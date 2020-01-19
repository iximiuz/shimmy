use std::ffi::CString;
use std::fs;
use std::io::Read;
use std::panic;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::str::FromStr;

use chrono::Utc;
use log::{debug, error, info, warn};
use nix::sys::signal::{
    Signal,
    Signal::{SIGCHLD, SIGINT, SIGKILL, SIGQUIT, SIGTERM},
};
use nix::unistd::{execv, fork, ForkResult, Pid};
use structopt::StructOpt;
use syslog::{BasicLogger, Facility, Formatter3164};

use shimmy::container::server::Server as ContainerServer;
use shimmy::nixtools::misc::{
    _exit, session_start, set_child_subreaper, set_parent_death_signal, to_pipe_fd,
};
use shimmy::nixtools::process::{
    kill, KillResult, TerminationStatus as ProcessTerminationStatus,
    TerminationStatus::{Exited, Signaled},
};
use shimmy::nixtools::signal::{signals_block, signals_restore, Signalfd};
use shimmy::nixtools::stdio::{create_pipes, set_stdio};
use shimmy::runtime::{await_runtime_termination, TerminationStatus as RuntimeTerminationStatus};
use shimmy::syncpipe::SyncPipe;

#[derive(Debug, StructOpt)]
#[structopt(name = "shimmy", about = "shimmy command line arguments")]
struct CliOpt {
    #[structopt(long = "shimmy-pidfile", parse(from_os_str))]
    pidfile: PathBuf,

    #[structopt(long = "shimmy-log-level", default_value = "INFO", parse(try_from_str = log::LevelFilter::from_str))]
    loglevel: log::LevelFilter,

    /// sync pipe file descriptor
    #[structopt(long = "syncpipe-fd", env = "_OCI_SYNCPIPE")]
    syncpipe_fd: i32,

    /// runtime executable path (eg. /usr/bin/runc)
    #[structopt(long = "runtime", parse(from_os_str))]
    runtime_path: PathBuf,

    #[structopt(long = "runtime-arg", multiple = true)]
    runtime_args: Vec<String>,

    #[structopt(long = "bundle", parse(from_os_str))]
    bundle: PathBuf,

    #[structopt(long = "container-id")]
    container_id: String,

    #[structopt(long = "container-pidfile", parse(from_os_str))]
    container_pidfile: PathBuf,

    #[structopt(long = "container-logfile", parse(from_os_str))]
    container_logfile: PathBuf,

    #[structopt(long = "container-exitfile", parse(from_os_str))]
    container_exitfile: PathBuf,

    #[structopt(long = "container-attachfile", parse(from_os_str))]
    container_attachfile: PathBuf,

    #[structopt(long = "stdin")]
    stdin: bool,

    #[structopt(long = "stdin-once")]
    stdin_once: bool,
}

fn main() {
    // Main process
    let opt = CliOpt::from_args();

    setup_logger(opt.loglevel);
    info!("[main] shimmy says hi!");

    match fork() {
        Ok(ForkResult::Parent { child }) => {
            // Main process (cont.)
            write_runtime_pidfile(opt.pidfile, child);
            exit(0);
        }
        Ok(ForkResult::Child) => (), // Shim process
        Err(err) => panic!("fork() of the shim process failed: {}", err),
    };

    // Shim process (cont.)
    debug!("[shim] initializing...");

    set_stdio((None, None, None));
    session_start();
    set_child_subreaper();

    let oldmask = signals_block(&[SIGCHLD, SIGINT, SIGQUIT, SIGTERM]);
    let (iomaster, ioslave) = create_pipes(opt.stdin, true, true);

    let runtime_pid = match fork() {
        Ok(ForkResult::Parent { child }) => child,
        Ok(ForkResult::Child) => {
            // Container runtime process
            debug!("[runtime] I've been forked!");

            // This will kill only runc top process (if it's still alive).
            // Forked by runc processes (i.e. init and container itself)
            // will not be affected (for better or for worse).
            set_parent_death_signal(SIGKILL);

            signals_restore(&oldmask);
            set_stdio(ioslave.streams());
            exec_oci_runtime(RuntimeCommand {
                runtime_path: opt.runtime_path,
                runtime_args: opt.runtime_args,
                container_id: opt.container_id,
                pidfile: opt.container_pidfile,
                bundle: opt.bundle,
            });
            _exit(127);
        }
        Err(err) => panic!("fork() of the container runtime process failed: {}", err),
    };

    // Shim process (cont.)
    drop(ioslave);

    let mut sigfd = Signalfd::new(&[SIGCHLD, SIGINT, SIGQUIT, SIGTERM]);
    match await_runtime_termination(&mut sigfd, runtime_pid) {
        RuntimeTerminationStatus::Solitary(Exited(.., 0), inflight) => {
            debug!("[shim] runtime terminated normally");

            let container_pid = read_container_pidfile(opt.container_pidfile);

            // Make sure we are ready to serve container
            // before reporting so back to the manager
            // (i.e. attach socket is ready, logger is ready, etc).
            let mut container_server = ContainerServer::new(
                container_pid,
                opt.container_attachfile,
                opt.container_logfile,
                iomaster.streams(),
                opt.stdin_once,
                sigfd,
            );

            SyncPipe::new(to_pipe_fd(opt.syncpipe_fd)).report_container_pid(container_pid);

            if let Some(sig) = inflight {
                deliver_inflight_signal(container_pid, sig);
            }

            save_container_termination_status(opt.container_exitfile, container_server.run());
        }

        ts => {
            warn!("[shim] runtime terminated abnormally: {}", ts);
            let mut buf = Vec::new();
            if let (_, _, Some(mut stderr)) = iomaster.streams() {
                if let Err(err) = stderr.read_to_end(&mut buf) {
                    warn!("[shim] failed to read runtime's STDERR: {}", err);
                }
            }
            SyncPipe::new(to_pipe_fd(opt.syncpipe_fd))
                .report_abnormal_runtime_termination(ts, &buf);
        }
    }

    info!("[shim] shimmy says bye!");
}

fn deliver_inflight_signal(container_pid: Pid, signal: Signal) {
    match kill(container_pid, signal) {
        Ok(KillResult::Delivered) => (),
        Ok(KillResult::ProcessNotFound) => {
            warn!("Failed to deliver inflight signal to container, probably exited")
        }
        Err(err) => warn!("Failed to deliver inflight signal to container: {}", err),
    }
}

fn save_container_termination_status<P: AsRef<Path>>(
    filename: P,
    status: ProcessTerminationStatus,
) {
    debug!(
        "[shim] saving container termination status [{}] to {}",
        status,
        filename.as_ref().display()
    );

    let now = Utc::now().to_rfc3339();
    let message = match status {
        Exited(.., code) => format!(
            r#"{{"at": "{}", "reason": "exited", "exitCode": {}}}"#,
            now, code,
        ),
        Signaled(.., sig) => format!(
            r#"{{"at": "{}", "reason": "signaled", "signal": {}}}"#,
            now, sig as libc::c_int,
        ),
    };
    if let Err(err) = fs::write(&filename, message) {
        panic!(
            "write() to container exit file {} failed: {}",
            filename.as_ref().display(),
            err
        )
    }
}

fn read_container_pidfile<P: AsRef<Path>>(filename: P) -> Pid {
    let content = fs::read_to_string(&filename).expect("fs::read_to_string() failed");
    return Pid::from_raw(
        content
            .parse::<i32>()
            .expect("failed to parse container PID file"),
    );
}

fn write_runtime_pidfile<P: AsRef<Path>>(filename: P, pid: Pid) {
    debug!(
        "[main] writing shim PID {} to {}",
        pid,
        filename.as_ref().display()
    );

    if let Err(err) = fs::write(&filename, format!("{}", pid)) {
        panic!("write() to {} failed: {}", filename.as_ref().display(), err)
    }
}

struct RuntimeCommand {
    runtime_path: PathBuf,
    runtime_args: Vec<String>,
    container_id: String,
    pidfile: PathBuf,
    bundle: PathBuf,
}

impl RuntimeCommand {
    fn to_argv(&self) -> Vec<CString> {
        let mut argv = Vec::new();
        argv.push(CString::new(self.runtime_path.to_str().unwrap()).unwrap());

        for arg in self.runtime_args.iter() {
            argv.push(CString::new(arg.trim_matches('\'')).unwrap());
        }

        argv.push(CString::new("create").unwrap());
        argv.push(CString::new("--bundle").unwrap());
        argv.push(CString::new(self.bundle.to_str().unwrap()).unwrap());
        argv.push(CString::new("--pid-file").unwrap());
        argv.push(CString::new(self.pidfile.to_str().unwrap()).unwrap());
        argv.push(CString::new(self.container_id.as_str()).unwrap());

        return argv;
    }
}

fn exec_oci_runtime(cmd: RuntimeCommand) {
    let argv = cmd.to_argv();
    debug!("[runtime] execing runc: {:?}", &argv);

    if let Err(err) = execv(&argv[0], &argv) {
        panic!("execv() failed: {}", err);
    }
}

fn setup_logger(level: log::LevelFilter) {
    let formatter = Formatter3164 {
        facility: Facility::LOG_USER,
        hostname: None,
        process: "shimmy".into(),
        pid: 0,
    };

    let logger = syslog::unix(formatter).expect("could not connect to syslog");
    log::set_boxed_logger(Box::new(BasicLogger::new(logger)))
        .map(|()| log::set_max_level(level))
        .expect("log::set_boxed_logger() failed");

    panic::set_hook(Box::new(|info| {
        error!("{}", info);
    }));
}
