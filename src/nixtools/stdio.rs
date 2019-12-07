use std::fs::File;
use std::io::{self, Read};
use std::mem;
use std::os::unix::io::{FromRawFd, RawFd};

use log::error;
use mio::event::Evented;
use mio::unix::EventedFd;
use mio::{Poll, PollOpt, Ready, Token};
use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::{close, dup2};

use crate::nixtools::pipe::Pipe;

#[derive(Debug)]
pub enum IOStream {
    DevNull,
    Fd(RawFd),
}

impl IOStream {
    pub fn read(&self, buf: &mut [u8]) -> usize {
        if let Self::Fd(fd) = self {
            let mut file = unsafe { File::from_raw_fd(*fd) };
            let nread = file.read(buf).expect("read() failed");
            mem::forget(file); // omit the destruciton of the file, i.e. no call to close(fd).
            nread
        } else {
            0
        }
    }

    pub fn read_all(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        if let Self::Fd(fd) = self {
            let mut file = unsafe { File::from_raw_fd(*fd) };
            file.read_to_end(&mut buf).expect("read_to_end() failed");
            mem::forget(file); // omit the destruciton of the file, i.e. no call to close(fd).
            buf
        } else {
            buf
        }
    }
}

impl Drop for IOStream {
    fn drop(&mut self) {
        if let Self::Fd(fd) = self {
            if let Err(err) = close(*fd) {
                error!("close(IOStream) failed: {}", err);
            }
        }
    }
}

impl Evented for IOStream {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        if let Self::Fd(fd) = self {
            EventedFd(fd).register(poll, token, interest, opts)
        } else {
            panic!("not implemented!");
        }
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        if let Self::Fd(fd) = self {
            EventedFd(fd).reregister(poll, token, interest, opts)
        } else {
            panic!("not implemented!");
        }
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        if let Self::Fd(fd) = self {
            EventedFd(fd).deregister(poll)
        } else {
            panic!("not implemented!");
        }
    }
}

#[allow(non_snake_case)]
pub struct IOStreams {
    pub In: IOStream,
    pub Out: IOStream,
    pub Err: IOStream,
}

pub fn set_stdio(streams: &IOStreams) {
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

        let slave = IOStreams {
            In: IOStream::DevNull,
            Out: IOStream::Fd(stdout.wr()),
            Err: IOStream::Fd(stderr.wr()),
        };
        let master = IOStreams {
            In: IOStream::DevNull,
            Out: IOStream::Fd(stdout.rd()),
            Err: IOStream::Fd(stderr.rd()),
        };

        // To prevent pipe objects calling close on underlying file descriptors:
        mem::forget(stdout);
        mem::forget(stderr);

        StdioPipes { slave, master }
    }
}
