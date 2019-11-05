use std::fs;
use std::path::Path;
use std::process::exit;

use libc::_exit;
use nix::sys::signal::Signal;
use nix::unistd::{fork, ForkResult, Pid};

mod nixtools;
use nixtools::{set_child_subreaper, set_parent_death_signal, setsid};

fn main() {
    // parse args

    match fork() {
        Err(err) => {
            panic!("fork() of the shim process failed: {}", err);
        }
        Ok(ForkResult::Parent { child, .. }) => {
            write_pid_file_and_exit("/home/vagrant/shimmy/pidfile.pid", child);
        }
        Ok(ForkResult::Child) => {
            setsid();
            set_child_subreaper();
            // make pipes for runc stdout/stderr
            // block SIGINT, SIGQUIT, SIGTERM

            match fork() {
                Err(err) => {
                    panic!("fork() of the container process failed: {}", err);
                }
                Ok(ForkResult::Parent { child, .. }) => {
                    // run server
                    //   read from stdout/stderr & dump to log
                    //   dump exit code on runc exit
                }
                Ok(ForkResult::Child) => {
                    set_parent_death_signal(Signal::SIGKILL); // TODO: check does it still work after exec)
                    // unblock signals
                    // dup std streams
                    exec_oci_runtime_or_exit();
                }
            }
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
    unsafe {
        _exit(127);
    }
}
