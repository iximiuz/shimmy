use std::fs;
use std::path::Path;
use std::process::exit;

use libc::_exit;
use nix::sys::signal::Signal::{SIGINT, SIGKILL, SIGQUIT, SIGTERM};
use nix::unistd::{fork, ForkResult, Pid};

mod nixtools;
use nixtools::{
    create_pipe, null_stdio_streams, session_start, set_child_subreaper, set_parent_death_signal,
    set_stdio_streams, signals_block, signals_restore, Pipe, STDIO,
};

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
            null_stdio_streams();
            session_start();
            set_child_subreaper();

            let pipes = create_runtime_stdio_pipes();
            let oldmask = signals_block(&[SIGINT, SIGQUIT, SIGTERM]);

            match fork() {
                Ok(ForkResult::Child) => {
                    // Container runtime process
                    // TODO: check does it still work after exec)
                    set_parent_death_signal(SIGKILL);
                    signals_restore(&oldmask);
                    set_stdio_streams(STDIO {
                        IN: Some(pipes.stdin.rd()),
                        OUT: Some(pipes.stdout.wr()),
                        ERR: Some(pipes.stderr.wr()),
                    });
                    exec_oci_runtime_or_die();
                }
                Ok(ForkResult::Parent { .. }) => {
                    // Shim process
                    start_shim_server(pipes);
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

struct Pipes {
    stdin: Pipe,
    stdout: Pipe,
    stderr: Pipe,
}

fn create_runtime_stdio_pipes() -> Pipes {
    Pipes {
        stdin: create_pipe(),
        stdout: create_pipe(),
        stderr: create_pipe(),
    }
}

fn start_shim_server(_runtime_stdio: Pipes) {
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
    unsafe {
        _exit(127);
    }
}
