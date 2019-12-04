use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::str::FromStr;

use libc::_exit;
use log::{debug, info, warn};
use nix::sys::signal::Signal;
use nix::sys::signal::Signal::{SIGCHLD, SIGINT, SIGKILL, SIGQUIT, SIGTERM};
use nix::unistd::{execv, fork, ForkResult, Pid};
use structopt::StructOpt;
use syslog::{BasicLogger, Facility, Formatter3164};

use shimmy::nixtools::misc::{
    get_pipe_fd_from_env, session_start, set_child_subreaper, set_parent_death_signal,
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

    /// container pidfile
    #[structopt(long = "runtime", short = "r", parse(from_os_str))]
    runtime_path: PathBuf,

    #[structopt(long = "runtime-arg", multiple = true)]
    runtime_args: Vec<String>,

    /// container pidfile
    #[structopt(long = "container-pidfile", short = "p", parse(from_os_str))]
    container_pidfile: PathBuf,

    /// container logfile
    #[structopt(long = "container-log-path", short = "l", parse(from_os_str))]
    container_logfile: PathBuf,

    /// container id
    #[structopt(long = "cid", short = "c")]
    container_id: String,

    /// container bundle path
    #[structopt(long = "bundle", short = "b", parse(from_os_str))]
    bundle: PathBuf,
}

fn main() {
    // Main process
    let opt = CliOpt::from_args();

    setup_logger(opt.loglevel);
    info!("shimmy says hi!");

    match fork() {
        Ok(ForkResult::Parent { child }) => {
            // Main process (cont.)
            write_runtime_pidfile_and_exit(opt.pidfile, child);
        }
        Ok(ForkResult::Child) => (), // Shim process
        Err(err) => panic!("fork() of the shim process failed: {}", err),
    };

    // Shim process (cont.)
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
            // TODO: check does it still work after exec)
            set_parent_death_signal(SIGKILL);
            signals_restore(&oldmask);
            set_stdio(iopipes.slave);
            exec_oci_runtime_or_exit(opt.runtime_path, opt.runtime_args);
            unreachable!();
        }
        Err(err) => panic!("fork() of the container runtime process failed: {}", err),
    };

    // Shim process (cont.)
    iopipes.slave.close_all();

    run_shim(runtime_pid, iopipes.master, opt.container_pidfile);
    info!("shimmy says bye!");
}

fn run_shim(runtime_pid: Pid, runtime_stdio: IOStreams, container_pidfile: PathBuf) {
    use ProcessTerminationStatus::Exited;

    let mut sigfd = Signalfd::new(&[SIGCHLD, SIGINT, SIGQUIT, SIGTERM]);
    let mut syncpipe = SyncPipe::new(get_pipe_fd_from_env("_OCI_SYNCPIPE"));

    match await_runtime_termination(&mut sigfd, runtime_pid) {
        RuntimeTerminationStatus::Solitary(Exited(.., 0), inflight) => {
            debug!("runtime terminated normally");

            let container_pid = read_container_pidfile(container_pidfile);
            syncpipe.report_container_pid(container_pid);

            if let Some(sig) = inflight {
                deliver_inflight_signal(container_pid, sig);
            }

            save_container_termination_status(serve_container(
                &mut sigfd,
                container_pid,
                runtime_stdio,
            ));
        }

        ts @ RuntimeTerminationStatus::Solitary(..) => {
            warn!("runtime terminated abnormally: {}", ts);
            syncpipe.report_abnormal_runtime_termination(ts, &runtime_stdio.Err.read_all());
        }

        ts @ RuntimeTerminationStatus::Conjoint(..) => {
            warn!("runtime and container terminated unexpectedly: {}", ts);
            syncpipe.report_abnormal_runtime_termination(ts, &runtime_stdio.Err.read_all());
        }
    }
}

fn serve_container(
    _sigfd: &mut Signalfd,
    container_pid: Pid,
    _container_stdio: IOStreams,
) -> ProcessTerminationStatus {
    // set up container execution timeout if needed
    // mio->run();
    // drain master stdio
    ProcessTerminationStatus::Exited(container_pid, 0)
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

fn save_container_termination_status(_status: ProcessTerminationStatus) {}

fn read_container_pidfile<P: AsRef<Path>>(filename: P) -> Pid {
    let content = fs::read_to_string(&filename).expect("fs::read_to_string() failed");
    return Pid::from_raw(
        content
            .parse::<i32>()
            .expect("failed to parse container PID file"),
    );
}

fn write_runtime_pidfile_and_exit<P: AsRef<Path>>(filename: P, pid: Pid) {
    if let Err(err) = fs::write(&filename, format!("{}", pid)) {
        panic!(
            "write() to pidfile {} failed: {}",
            filename.as_ref().to_string_lossy(),
            err
        )
    }
    exit(0);
}

fn exec_oci_runtime_or_exit(runtime_path: PathBuf, runtime_args: Vec<String>) {
    let mut args = Vec::new();
    args.push(CString::new(runtime_path.to_str().unwrap()).unwrap());
    for arg in runtime_args {
        args.push(CString::new(arg).unwrap());
    }
    args.push(CString::new("create").unwrap());

    if let Err(err) = execv(&args[0], &args) {
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
}
