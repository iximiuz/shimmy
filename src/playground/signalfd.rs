use std::process::exit;
use std::thread::sleep;
use std::time::Duration;

use libc;
use nix::{self};
use nix::sys::signal::Signal::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM};
use nix::sys::signalfd::*;
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::{fork, getpid, ForkResult, Pid};

use shimmy::nixtools::{set_child_subreaper, signals_block};

fn main() {
    println!("Hi there! My pid is {}", getpid());

    set_child_subreaper();
    signals_block(&[SIGCHLD, SIGINT, SIGQUIT, SIGTERM]);
    println!("Signals have been blocked! Waiting for 10 seconds...");

    match fork() {
        Ok(ForkResult::Parent {..}) => (),
        Ok(ForkResult::Child) => {
            println!("[child] Hi there! My pid is {}", getpid());
            match fork() {
                Ok(ForkResult::Parent {..}) => (),
                Ok(ForkResult::Child) => {
                    println!("[grandchild] Hi there! My pid is {}", getpid());
                    exit(124);
                },
                Err(err) => panic!("fork() failed {}", err),
            };
            exit(123);
        },
        Err(err) => panic!("fork() failed {}", err),
    };

    sleep(Duration::from_millis(10000));

    let mut mask = SigSet::empty();
    mask.add(signal::SIGCHLD);
    mask.add(signal::SIGINT);
    mask.add(signal::SIGQUIT);
    mask.add(signal::SIGTERM);
    mask.thread_block().unwrap();

    // let mut sfd = SignalFd::new(&mask).unwrap();
    let mut sfd = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK).unwrap();

    loop {
        match sfd.read_signal() {
            Ok(Some(sig)) => {
                println!("Got a signal {:?}", sig);
                while sig.ssi_signo == SIGCHLD as u32 {
                    match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                        Ok(res) => println!("waitpid() returned {:?}", res),
                        Err(nix::Error::Sys(errno)) => {
                            if errno as libc::c_int == libc::ECHILD {
                                break;
                            }
                            panic!("waitpid() failed {:?}", errno);
                        },
                        Err(err) => panic!("waitpid() failed {:?}", err),
                    }
                }
            },
            Ok(None) => break,
            Err(err) => panic!("read(signalfd) failed {}", err),
        }
    }
}
