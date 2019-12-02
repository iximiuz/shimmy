use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::process::exit;

extern crate syslog;
#[macro_use]
extern crate log;

use libc::_exit;
use nix::sys::signal::Signal;
use nix::sys::signal::Signal::{SIGCHLD, SIGINT, SIGKILL, SIGQUIT, SIGTERM};
use nix::unistd::{execv, fork, ForkResult, Pid};
use syslog::{BasicLogger, Facility, Formatter3164};

use shimmy::nixtools::kill::{get_child_termination_status, kill, KillResult, TerminationStatus};
use shimmy::nixtools::misc::{
    get_pipe_fd_from_env, session_start, set_child_subreaper, set_parent_death_signal,
};
use shimmy::nixtools::signal::{signals_block, signals_restore, Signalfd};
use shimmy::nixtools::stdio::{set_stdio, IOStream, IOStreams, StdioPipes};
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
        RuntimeTermination::Normal { inflight } => {
            // syncfd.report_container_pid();
            shim_server_run(sigfd, runtime_stdio, inflight);
            // drain master stdio
        }

        RuntimeTermination::NormalWithContainer { container } => {
            // syncfd.report_container_pid();
            // drain master stdio
        }

        RuntimeTermination::Abnormal { runtime } => {
            syncpipe.write_abnormal_runtime_termination(runtime, &runtime_stdio.Err.read_all());
        }
    }
}

fn shim_server_run(sigfd: Signalfd, container_stdio: IOStreams, inflight_signal: Option<Signal>) {
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

enum RuntimeTermination {
    Normal { inflight: Option<Signal> },
    NormalWithContainer { container: TerminationStatus },
    Abnormal { runtime: TerminationStatus },
}

fn await_runtime_termination(sigfd: &mut Signalfd, runtime_pid: Pid) -> RuntimeTermination {
    let mut container: Option<TerminationStatus> = None;
    let mut inflight: Option<Signal> = None;
    loop {
        match sigfd.read_signal() {
            SIGCHLD => {
                debug!("SIGCHLD received, querying runtime status.");

                match get_termination_statuses(runtime_pid) {
                    (Some(rt), ct) => {
                        assert!(
                            ct.is_none() || container.is_none(),
                            "ambiguous container termination status"
                        );
                        return dispatch_runtime_termination(rt, container.or(ct), inflight);
                    }

                    (None, Some(ct)) => {
                        assert!(
                            container.is_none(),
                            "ambiguous container termination status"
                        );
                        container = Some(ct); // Keep it for later use.
                    }

                    (None, None) => (), // Continue...
                }
            }

            sig if [SIGINT, SIGQUIT, SIGTERM].contains(&sig) => {
                debug!("{} received, propagating to runtime.", sig);

                match kill(runtime_pid, sig) {
                    Ok(KillResult::Delivered) => (), // Keep waiting for runtime termination...

                    Ok(KillResult::ProcessNotFound) => {
                        // Runtime already has exited, keep the signal
                        // to send it to the container once we know its PID.
                        inflight = Some(sig);
                    }

                    Err(err) => panic!("kill(runtime_pid, {}) failed: {:?}", sig, err),
                }
            }
            sig => panic!("unexpected signal received {:?}", sig),
        };
    }
}

fn get_termination_statuses(
    runtime_pid: Pid,
) -> (Option<TerminationStatus>, Option<TerminationStatus>) {
    let mut runtime: Option<TerminationStatus> = None;
    let mut container: Option<TerminationStatus> = None;

    while let Some(status) = get_child_termination_status() {
        if status.pid() == runtime_pid {
            assert!(runtime.is_none());
            runtime = Some(status);
        } else {
            assert!(container.is_none());
            container = Some(status);
        }
    }

    (runtime, container)
}

fn dispatch_runtime_termination(
    runtime: TerminationStatus,
    container: Option<TerminationStatus>,
    inflight: Option<Signal>,
) -> RuntimeTermination {
    if runtime.exited_with_code(0) {
        match container {
            None => RuntimeTermination::Normal { inflight },

            Some(container) => {
                if let Some(sig) = inflight {
                    warn!(
                        "{} received but both runtime and container have been already terminated",
                        sig
                    );
                }
                RuntimeTermination::NormalWithContainer { container }
            }
        }
    } else {
        RuntimeTermination::Abnormal { runtime }
    }
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
    if let Err(err) = execv(&name, &vec![name.clone()]) {
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
