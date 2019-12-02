use std::fs::OpenOptions;
use std::io::prelude::*;
use std::str;
use std::thread::sleep;
use std::time::Duration;

use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::{close, dup2, fork, getpid, getppid, pipe, read, ForkResult};

fn main() {
    println!("[main] Hi there! My pid is {}", getpid());

    let (pr, pw) = match pipe() {
        Ok((pr, pw)) => (pr, pw),
        Err(err) => panic!("[main] pipe() failed: {}", err),
    };

    match fork() {
        Ok(ForkResult::Parent { child, .. }) => {
            println!("[main] Forked new child with pid {}", child);
        }
        Ok(ForkResult::Child) => {
            close(pr).expect("[main] close(pw) failed");

            let dev_null_r = open("/dev/null", OFlag::O_RDONLY, Mode::empty()).unwrap();
            dup2(dev_null_r, 0).expect("dup2(STDIN) failed");
            dup2(pw, 1).expect("dup2(STDOUT) failed");
            dup2(pw, 2).expect("dup2(STDERR) failed");

            let mut log = OpenOptions::new()
                .create(true)
                .write(true)
                .append(true)
                .open("/home/vagrant/shimmy/child.log")
                .unwrap();
            loop {
                let msg = format!(
                    "[child] I'm alive! My PID is {} and PPID is {}.",
                    getpid(),
                    getppid()
                );
                println!("{}", msg);
                writeln!(log, "{}", msg).expect("[child] writeln!(log) failed");
                sleep(Duration::from_millis(500));
            }
        }
        Err(err) => panic!("[main] fork() failed: {}", err),
    };

    close(pw).expect("[main] close(pw) failed");

    for _ in 1..10 {
        let mut buf = vec![0; 1024];
        let nread = read(pr, buf.as_mut_slice()).unwrap();
        println!(
            "[main] read {} bytes: [{}]",
            nread,
            str::from_utf8(&buf).unwrap()
        );
        sleep(Duration::from_millis(900));
    }
}
