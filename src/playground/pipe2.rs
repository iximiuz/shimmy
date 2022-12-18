use std::str;
use std::thread::sleep;
use std::time::Duration;

use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use nix::sys::wait::waitpid;
use nix::unistd::{close, dup2, fork, getpid, getppid, pipe, read, ForkResult, Pid};

fn main() {
    println!("[main] Hi there! My pid is {}", getpid());

    let (pr, pw) = match pipe() {
        Ok((pr, pw)) => (pr, pw),
        Err(err) => panic!("[main] pipe() failed: {}", err),
    };

    match unsafe { fork() } {
        Ok(ForkResult::Parent { child, .. }) => {
            println!("[main] Forked new child with pid {}", child);
        }
        Ok(ForkResult::Child) => {
            close(pr).expect("[main] close(pw) failed");
            dup2(pw, 1).expect("dup2(STDOUT) failed");

            let dev_null_r = open("/dev/null", OFlag::O_RDONLY, Mode::empty()).unwrap();
            dup2(dev_null_r, 0).expect("dup2(STDIN) failed");

            let dev_null_w = open("/dev/null", OFlag::O_WRONLY, Mode::empty()).unwrap();
            dup2(dev_null_w, 2).expect("dup2(STDERR) failed");

            let msg = format!(
                "[child] I'm alive! My PID is {} and PPID is {}.",
                getpid(),
                getppid()
            );
            println!("{}", msg);
        }
        Err(err) => panic!("[main] fork() failed: {}", err),
    };

    close(pw).expect("[main] close(pw) failed");
    waitpid(Pid::from_raw(-1), None).expect("waitpid() failed");

    for _ in 1..1024 {
        let mut buf = vec![0; 54];
        let nread = read(pr, buf.as_mut_slice()).unwrap();
        println!(
            "[main] read {} bytes: [{}]",
            nread,
            str::from_utf8(&buf).unwrap()
        );
        sleep(Duration::from_millis(100));
    }
}
