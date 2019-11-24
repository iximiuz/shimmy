use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::process::exit;
use std::str;

extern crate syslog;
#[macro_use]
extern crate log;

use libc::_exit;
use nix::sys::signal::SigSet;
use nix::sys::signal::Signal::{SIGCHLD, SIGINT, SIGKILL, SIGQUIT, SIGTERM};
use nix::sys::signalfd::*;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{execv, fork, read, ForkResult, Pid};
use syslog::{Facility, Formatter3164, BasicLogger};

use shimmy::nixtools::{
    session_start, set_child_subreaper, set_parent_death_signal, set_stdio, signals_block,
    signals_restore, IOStream, IOStreams,
};
use shimmy::stdiotools::StdioPipes;

fn main() {
    // Main process
    setup_logger();
    info!("shimmy says hi!");

    // TODO: parse args

    match fork() {
        Ok(ForkResult::Parent { child, .. }) => {
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

    let iopipes = StdioPipes::new();
    let oldmask = signals_block(&[SIGCHLD, SIGINT, SIGQUIT, SIGTERM]);

    let runtime_pid = match fork() {
        Ok(ForkResult::Parent { child, .. }) => child, 
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
    shim_server_run(runtime_pid, iopipes.master);
}

struct State {
    runtime_pid: Pid,
    runtime_status: Option<i32>,
    container_pid: Option<Pid>,
    container_status: Option<i32>,
    inflight_signal: Option<i32>,
}

impl State {
    fn new(runtime_pid: Pid) -> Self {
        Self {
            runtime_pid: runtime_pid,
            runtime_status: None,
            container_pid: None,
            container_status: None,
            inflight_signal: None
        }
    }
}

fn shim_server_run(runtime_pid: Pid, runtime_stdio: IOStreams) {
    // create signalfd and start reading
    // wait for sigchld
    //
    // let sig = read(sfd)
    // if sig in (SIGINT, SIGQUIT, SIGTERM):
    //     if runtime_pid:
    //         kill(runtime_pid, sig)
    //     if container_pid:
    //         kill(container_pid, sig)
    //
    // if sig == SIGCHLD:
    //    let pid1 = waitpid(-1, &status, WNOHANG);
    //    if pid1 == runtime_pid:
    //        if status == 0:
    //            runtime_pid = 0
    //        else:
    //            do a cleanup, report back to the parent, exit
    //
    //    if pid1 != runtime_pid:
    //        container_pid = pid

    let state = State::new(runtime_pid);

    let mut sigmask = SigSet::empty();
    sigmask.add(SIGCHLD);
    sigmask.add(SIGINT);
    sigmask.add(SIGQUIT);
    sigmask.add(SIGTERM);
    let mut sfd = SignalFd::new(&sigmask).unwrap();
    match sfd.read_signal() {
        Ok(Some(sig)) => {
            debug!("got signal {:?}", sig);
            if sig.ssi_signo == SIGCHLD as u32 {
                let (pid, exit_code) = wait_child().unwrap();
                debug!("something exited PID={}, exit_code={}", pid, exit_code);
                
                while let Some((pid, exit_code)) = wait_child() {
                    debug!("fuck this shit PID={}, exit_code={}", pid, exit_code);
                }
            } else {
                // SIGINT, SIGQUIT, SIGTERM
                // state.inflight_signal = Some(sig.ssi_signo);
            }
        }
        Ok(None) => panic!("wtf? We are in blocking mode"),
        Err(err) => panic!("read(signalfd) failed {}", err),
    };

    // TODO:
    // waitpid(runtime_pid)
    // if runtime exit code != 0 report error (using ENV[SYNC_FD]) and exit
    // read container pid file (runc was supposed to store it on disk before exiting)
    // report container pid back to the parent (using ENV[SYNC_FD])
    // waitpid(container pid)
    // set up container execution timeout if needed
    // mio->run();
    // report container exit code to the parent (using ENV[SYNC_FD])
    if let IOStream::Fd(fd) = runtime_stdio.Err {
        let mut buf = vec![0; 1024];
        let nread = read(fd, buf.as_mut_slice()).unwrap();
        debug!(
            "[server] read (STDERR) {} bytes: [{}]",
            nread,
            str::from_utf8(&buf).unwrap()
        );
    }

    if let IOStream::Fd(fd) = runtime_stdio.Out {
        let mut buf = vec![0; 1024];
        let nread = read(fd, buf.as_mut_slice()).unwrap();
        debug!(
            "[server] read (STDOUT) {} bytes: [{}]",
            nread,
            str::from_utf8(&buf).unwrap()
        );
    }
}

fn wait_child() -> Option<(Pid, i32)> {
    match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
        Ok(WaitStatus::Exited(pid, exit_code)) => {
            return Some((pid, exit_code));
        }
        Ok(WaitStatus::StillAlive) => {
            return None;
        }
        Ok(res) => {
            panic!("waitpid() returned unexpected result {:?}", res);
        }
        Err(nix::Error::Sys(errno)) => {
            if errno as libc::c_int != libc::ECHILD {
                panic!("waitpid() returned unexpected error {:?}", errno);
            }
            return None;
        }
        Err(err) => {
            panic!("waitpid() failed {:?}", err);
        }
    };
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
            .map(|()| log::set_max_level(log::LevelFilter::Debug)).expect("log::set_boxed_logger() failed");
}
