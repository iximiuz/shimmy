use std::fs;
use std::path::Path;
use std::process::exit;

use libc::_exit;
use nix::fcntl::{OFlag, open};
use nix::sys::signal::Signal::{SIGINT, SIGKILL, SIGQUIT, SIGTERM};
use nix::sys::stat::Mode;
use nix::unistd::{ForkResult, Pid, dup2, fork};

mod nixtools;
use nixtools::{
    set_child_subreaper, set_parent_death_signal, setsid, signals_block, signals_restore,
};

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
            null_std_streams();
            setsid();
            set_child_subreaper();
            // make pipes for runc stdout/stderr
            let oldmask = signals_block(&[SIGINT, SIGQUIT, SIGTERM]);

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
                    set_parent_death_signal(SIGKILL); // TODO: check does it still work after exec)
                    signals_restore(&oldmask);
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

fn null_std_streams() {
    let dev_null_r = open("/dev/null", OFlag::O_RDONLY, Mode::empty()).expect("oops");
    let dev_null_w = open("/dev/null", OFlag::O_WRONLY, Mode::empty()).expect("oops");

    dup2(dev_null_r, 0).expect("oops");
    dup2(dev_null_w, 1).expect("oops");
    dup2(dev_null_w, 2).expect("oops");
}
