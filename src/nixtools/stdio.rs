use std::os::unix::io::RawFd;

use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::{close, dup2};

use crate::nixtools::pipe::Pipe;

pub enum IOStream {
    DevNull,
    Fd(RawFd),
}

#[allow(non_snake_case)]
pub struct IOStreams {
    pub In: IOStream,
    pub Out: IOStream,
    pub Err: IOStream,
}

impl IOStreams {
    pub fn close_all(&self) {
        if let IOStream::Fd(fd) = self.In {
            close(fd).expect("close(STDIN) failed");
        }
        if let IOStream::Fd(fd) = self.Out {
            close(fd).expect("close(STDOUT) failed");
        }
        if let IOStream::Fd(fd) = self.Err {
            close(fd).expect("close(STDERR) failed");
        }
    }
}

pub fn set_stdio(streams: IOStreams) {
    match streams.In {
        IOStream::Fd(fd) => {
            dup2(fd, 0).expect("dup2(fd, STDIN_FILENO) failed");
        }
        IOStream::DevNull => {
            let fd = open_dev_null(OFlag::O_RDONLY);
            dup2(fd, 0).expect("dup2(fd, STDIN_FILENO) failed");
            close(fd).expect("close('/dev/null (RDONLY)') failed");
        }
    }

    match streams.Out {
        IOStream::Fd(fd) => {
            dup2(fd, 1).expect("dup2(fd, STDOUT_FILENO) failed");
        }
        IOStream::DevNull => {
            let fd = open_dev_null(OFlag::O_WRONLY);
            dup2(fd, 1).expect("dup2(fd, STDOUT_FILENO) failed");
            close(fd).expect("close('/dev/null (WRONLY)') failed");
        }
    }

    match streams.Err {
        IOStream::Fd(fd) => {
            dup2(fd, 2).expect("dup2(fd, STDERR_FILENO) failed");
        }
        IOStream::DevNull => {
            let fd = open_dev_null(OFlag::O_WRONLY);
            dup2(fd, 2).expect("dup2(fd, STDERR_FILENO) failed");
            close(fd).expect("close('/dev/null (WRONLY)') failed");
        }
    }
}

fn open_dev_null(flags: OFlag) -> RawFd {
    open("/dev/null", flags | OFlag::O_CLOEXEC, Mode::empty()).expect(&format!(
        "open('/dev/null') for {} failed",
        human_readable_mode(flags)
    ))
}

fn human_readable_mode(flags: OFlag) -> &'static str {
    match flags {
        _ if flags & OFlag::O_RDONLY == OFlag::O_RDONLY => "reading",
        _ if flags & OFlag::O_WRONLY == OFlag::O_WRONLY => "writing",
        _ => unreachable!(),
    }
}

pub struct StdioPipes {
    pub slave: IOStreams,
    pub master: IOStreams,
}

impl StdioPipes {
    pub fn new() -> StdioPipes {
        let stdout = Pipe::new();
        let stderr = Pipe::new();
        StdioPipes {
            slave: IOStreams {
                In: IOStream::DevNull,
                Out: IOStream::Fd(stdout.wr()),
                Err: IOStream::Fd(stderr.wr()),
            },
            master: IOStreams {
                In: IOStream::DevNull,
                Out: IOStream::Fd(stdout.rd()),
                Err: IOStream::Fd(stderr.rd()),
            },
        }
    }
}
