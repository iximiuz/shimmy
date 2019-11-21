use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::process::exit;
use std::str;

use libc::_exit;
use nix::sys::signal::Signal::{SIGINT, SIGKILL, SIGQUIT, SIGTERM};
use nix::unistd::{execv, fork, read, ForkResult, Pid};

mod nixtools;
use nixtools::{
    session_start, set_child_subreaper, set_parent_death_signal, set_stdio, signals_block,
    signals_restore, IOStream, IOStreams,
};
mod stdiotools;
use stdiotools::StdioPipes;

fn main() {
    // Main process
    // TODO: parse args

    match fork() {
        Ok(ForkResult::Parent { child, .. }) => {
            // Main process
            write_pid_file_and_exit("/home/vagrant/shimmy/pidfile.pid", child);
        }
        Ok(ForkResult::Child) => {
            // Shim process
            set_stdio(IOStreams {
                In: IOStream::DevNull,
                Out: IOStream::DevNull,
                Err: IOStream::DevNull,
            });
            session_start();
            set_child_subreaper();

            let iopipes = StdioPipes::new();
            let oldmask = signals_block(&[SIGINT, SIGQUIT, SIGTERM]);

            match fork() {
                Ok(ForkResult::Child) => {
                    // Container runtime process
                    // TODO: check does it still work after exec)
                    set_parent_death_signal(SIGKILL);
                    signals_restore(&oldmask);
                    set_stdio(iopipes.slave);
                    exec_oci_runtime_or_die();
                }
                Ok(ForkResult::Parent { .. }) => {
                    // Shim process
                    // TODO: set exit signal handlers before restoring the mask
                    signals_restore(&oldmask);
                    iopipes.slave.close_all();
                    start_shim_server(iopipes.master);
                }
                Err(err) => {
                    panic!("fork() of the container runtime process failed: {}", err);
                }
            }
        }
        Err(err) => {
            panic!("fork() of the shim process failed: {}", err);
        }
    };
}

fn start_shim_server(runtime_stdio: IOStreams) {
    if let IOStream::Fd(fd) = runtime_stdio.Err {
        let mut buf = vec![0; 1024];
        let nread = read(fd, buf.as_mut_slice()).unwrap();
        println!(
            "[server] read (STDERR) {} bytes: [{}]",
            nread,
            str::from_utf8(&buf).unwrap()
        );
    }

    if let IOStream::Fd(fd) = runtime_stdio.Out {
        let mut buf = vec![0; 1024];
        let nread = read(fd, buf.as_mut_slice()).unwrap();
        println!(
            "[server] read (STDOUT) {} bytes: [{}]",
            nread,
            str::from_utf8(&buf).unwrap()
        );
    }

    // run server
    //   read from stdout/stderr & dump to log
    //   dump exit code on runc exit
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

fn exec_oci_runtime_or_die() {
    let name = CString::new("/usr/bin/runc").unwrap();
    if let Err(err) = execv(&name, &vec![name.clone()]) {
        panic!("execv() failed: {}", err);
    }

    unsafe {
        _exit(127);
    }
}
