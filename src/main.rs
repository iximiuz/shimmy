use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::process::exit;

use libc::_exit;
use log::{debug, info, warn};
use nix::sys::signal::Signal;
use nix::sys::signal::Signal::{SIGCHLD, SIGINT, SIGKILL, SIGQUIT, SIGTERM};
use nix::unistd::{execv, fork, ForkResult, Pid};
use syslog::{BasicLogger, Facility, Formatter3164};

use shimmy::nixtools::misc::{
    get_pipe_fd_from_env, session_start, set_child_subreaper, set_parent_death_signal,
};
use shimmy::nixtools::process::TerminationStatus::Exited;
use shimmy::nixtools::signal::{signals_block, signals_restore, Signalfd};
use shimmy::nixtools::stdio::{set_stdio, IOStream, IOStreams, StdioPipes};
use shimmy::runtime::{await_runtime_termination, TerminationStatus};
use shimmy::syncpipe::SyncPipe;

fn main() {
    // Main process
    setup_logger();
    info!("shimmy says hi!");

    // TODO: parse args

    match fork() {
        Ok(ForkResult::Parent { child }) => {
            // Main process (cont.)
            write_pid_file_and_exit("/home/vagrant/shimmy/pidfile.pid", child);
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
            exec_oci_runtime_or_exit();
            unreachable!();
        }
        Err(err) => panic!("fork() of the container runtime process failed: {}", err),
    };

    // Shim process (cont.)
    iopipes.slave.close_all();

    run_shim(runtime_pid, iopipes.master);
    info!("shimmy says bye!");
}

fn run_shim(runtime_pid: Pid, runtime_stdio: IOStreams) {
    let mut sigfd = Signalfd::new(&[SIGCHLD, SIGINT, SIGQUIT, SIGTERM]);
    let mut syncpipe = SyncPipe::new(get_pipe_fd_from_env("_OCI_SYNCPIPE"));

    match await_runtime_termination(&mut sigfd, runtime_pid) {
        TerminationStatus::Solitary(Exited(.., 0), inflight) => {
            debug!("runtime terminated normally");
            // syncfd.report_container_pid();
            shim_server_run(sigfd, runtime_stdio, inflight);
            // drain master stdio
        }

        ts @ TerminationStatus::Solitary(..) => {
            warn!("runtime terminated abnormally: {}", ts);
            syncpipe.report_abnormal_runtime_termination(ts, &runtime_stdio.Err.read_all());
        }

        ts @ TerminationStatus::Conjoint(..) => {
            warn!("runtime and container terminated unexpectedly: {}", ts);
            syncpipe.report_abnormal_runtime_termination(ts, &runtime_stdio.Err.read_all());
        }
    }
}

fn shim_server_run(
    _sigfd: Signalfd,
    _container_stdio: IOStreams,
    _inflight_signal: Option<Signal>,
) {
    // TODO:
    // read container pid file (runc was supposed to store it on disk before exiting)
    // report container pid back to the parent (using ENV[SYNC_FD])

    // if have an inflight_signal, send it to container
    // set up container execution timeout if needed
    // mio->run();
    // report container exit code to the parent (using ENV[SYNC_FD])
    // if let IOStream::Fd(fd) = runtime_stdio.Err {
    //     let mut buf = vec![0; 1024];
    //     let nread = read(fd, buf.as_mut_slice()).unwrap();
    //     debug!(
    //         "[server] read (STDERR) {} bytes: [{}]",
    //         nread,
    //         str::from_utf8(&buf).unwrap()
    //     );
    // }

    // if let IOStream::Fd(fd) = runtime_stdio.Out {
    //     let mut buf = vec![0; 1024];
    //     let nread = read(fd, buf.as_mut_slice()).unwrap();
    //     debug!(
    //         "[server] read (STDOUT) {} bytes: [{}]",
    //         nread,
    //         str::from_utf8(&buf).unwrap()
    //     );
    // }
}

fn write_pid_file_and_exit<P: AsRef<Path>>(filename: P, pid: Pid) {
    if let Err(err) = fs::write(&filename, format!("{}", pid)) {
        panic!(
            "write() to pidfile {} failed: {}",
            filename.as_ref().to_string_lossy(),
            err
        )
    }
    exit(0);
}

fn exec_oci_runtime_or_exit() {
    let name = CString::new("/usr/bin/runc").unwrap();
    let arg1 = CString::new("-foobar").unwrap();
    if let Err(err) = execv(&name, &vec![name.clone(), arg1]) {
        panic!("execv() failed: {}", err);
    }

    unsafe {
        _exit(127);
    }
}

fn setup_logger() {
    let formatter = Formatter3164 {
        facility: Facility::LOG_USER,
        hostname: None,
        process: "shimmy".into(),
        pid: 0,
    };

    let logger = syslog::unix(formatter).expect("could not connect to syslog");
    log::set_boxed_logger(Box::new(BasicLogger::new(logger)))
        .map(|()| log::set_max_level(log::LevelFilter::Debug))
        .expect("log::set_boxed_logger() failed");
}
