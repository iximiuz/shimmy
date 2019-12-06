use std::ffi::CString;
use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::str::FromStr;

use libc::_exit;
use log::{debug, error, info, warn};
use nix::sys::signal::Signal;
use nix::sys::signal::Signal::{SIGCHLD, SIGINT, SIGKILL, SIGQUIT, SIGTERM};
use nix::unistd::{execv, fork, ForkResult, Pid};
use structopt::StructOpt;
use syslog::{BasicLogger, Facility, Formatter3164};

use shimmy::container::serve_container;
use shimmy::nixtools::misc::{
    session_start, set_child_subreaper, set_parent_death_signal, to_pipe_fd,
};
use shimmy::nixtools::process::{kill, KillResult, TerminationStatus as ProcessTerminationStatus};
use shimmy::nixtools::signal::{signals_block, signals_restore, Signalfd};
use shimmy::nixtools::stdio::{set_stdio, IOStream, IOStreams, StdioPipes};
use shimmy::runtime::{await_runtime_termination, TerminationStatus as RuntimeTerminationStatus};
use shimmy::syncpipe::SyncPipe;

#[derive(Debug, StructOpt)]
#[structopt(name = "shimmy", about = "shimmy command line arguments")]
struct CliOpt {
    /// shimmy pidfile
    #[structopt(long = "shimmy-pidfile", short = "P", parse(from_os_str))]
    pidfile: PathBuf,

    /// shimmy log level
    #[structopt(long = "shimmy-log-level", default_value = "INFO", parse(try_from_str = log::LevelFilter::from_str))]
    loglevel: log::LevelFilter,

    /// sync pipe file descriptor
    #[structopt(short = "S", long = "syncpipe-fd", env = "_OCI_SYNCPIPE")]
    syncpipe_fd: String,

    /// runtime executable path (eg. /usr/bin/runc)
    #[structopt(long = "runtime", short = "r", parse(from_os_str))]
    runtime_path: PathBuf,

    #[structopt(long = "runtime-arg", multiple = true)]
    runtime_args: Vec<String>,

    /// container bundle path
    #[structopt(long = "bundle", short = "b", parse(from_os_str))]
    bundle: PathBuf,

    /// container id
    #[structopt(long = "container-id", short = "c")]
    container_id: String,

    /// container pidfile
    #[structopt(long = "container-pidfile", short = "p", parse(from_os_str))]
    container_pidfile: PathBuf,

    /// container logfile
    #[structopt(long = "container-log-path", short = "l", parse(from_os_str))]
    container_logfile: PathBuf,

    /// container exit dir
    #[structopt(long = "container-exit-dir", short = "e", parse(from_os_str))]
    container_exit_dir: PathBuf,
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

    set_stdio(IOStreams {
        In: IOStream::DevNull,
        Out: IOStream::DevNull,
        Err: IOStream::DevNull,
    });
    session_start();
    set_child_subreaper();
    let oldmask = signals_block(&[SIGCHLD, SIGINT, SIGQUIT, SIGTERM]);
    let iopipes = StdioPipes::new();

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
            set_stdio(iopipes.slave);
            exec_oci_runtime_or_exit(RuntimeCommand {
                runtime_path: opt.runtime_path,
                runtime_args: opt.runtime_args,
                container_id: opt.container_id,
                pidfile: opt.container_pidfile,
                bundle: opt.bundle,
            });
            unreachable!();
        }
        Err(err) => panic!("fork() of the container runtime process failed: {}", err),
    };

    // Shim process (cont.)
    iopipes.slave.close_all();

    let mut sigfd = Signalfd::new(&[SIGCHLD, SIGINT, SIGQUIT, SIGTERM]);

    debug!("[shim] awaiting runtime termination...");
    if let Some(status) = run_shim(
        await_runtime_termination(&mut sigfd, runtime_pid),
        iopipes.master,
        opt.container_pidfile,
        sigfd,
        SyncPipe::new(to_pipe_fd(&opt.syncpipe_fd)),
    ) {
        save_container_termination_status(opt.container_exit_dir.join(opt.container_id), status);
    }

    info!("[shim] shimmy says bye!");
}

fn run_shim(
    runtime_status: RuntimeTerminationStatus,
    runtime_stdio: IOStreams,
    container_pidfile: PathBuf,
    mut sigfd: Signalfd,
    mut syncpipe: SyncPipe,
) -> Option<ProcessTerminationStatus> {
    use ProcessTerminationStatus::Exited;

    return match runtime_status {
        RuntimeTerminationStatus::Solitary(Exited(.., 0), inflight) => {
            debug!("[shim] runtime terminated normally");

            let container_pid = read_container_pidfile(container_pidfile);
            syncpipe.report_container_pid(container_pid);
            drop(syncpipe);

            if let Some(sig) = inflight {
                deliver_inflight_signal(container_pid, sig);
            }

            Some(serve_container(&mut sigfd, container_pid, runtime_stdio))
        }

        ts @ RuntimeTerminationStatus::Solitary(..) => {
            warn!("[shim] runtime terminated abnormally: {}", ts);
            syncpipe.report_abnormal_runtime_termination(ts, &runtime_stdio.Err.read_all());

            None
        }

        ts @ RuntimeTerminationStatus::Conjoint(..) => {
            warn!(
                "[shim] runtime and container terminated unexpectedly: {}",
                ts
            );
            syncpipe.report_abnormal_runtime_termination(ts, &runtime_stdio.Err.read_all());

            None
        }
    };
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
        "[shim] saving container termination status {} to {}",
        status,
        filename.as_ref().display()
    );

    if let Err(err) = fs::write(&filename, format!("{}", status)) {
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
        "[main] writing shim PID {} to pidfile {}",
        pid,
        filename.as_ref().display()
    );

    if let Err(err) = fs::write(&filename, format!("{}", pid)) {
        panic!(
            "write() to pidfile {} failed: {}",
            filename.as_ref().display(),
            err
        )
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

fn exec_oci_runtime_or_exit(cmd: RuntimeCommand) {
    let argv = cmd.to_argv();
    debug!("[runtime] execing runc: {:?}", &argv);

    if let Err(err) = execv(&argv[0], &argv) {
        error!("[runtime] execv() failed: {}", err);
        panic!("execv() failed: {}", err);
    }

    unsafe {
        _exit(127);
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
