use std::fs::File;
use std::io::{self, Read, Write};
use std::mem;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

use log::error;
use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::{close, dup2};

use crate::nixtools::pipe::Pipe;

enum Stream {
    DevNull,
    Fd(RawFd),
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Self::Fd(fd) = self {
            if let Err(err) = close(*fd) {
                error!("close({}) failed: {}", fd, err);
            }
        }
    }
}

pub struct IStream(Stream);

impl IStream {
    pub fn devnull() -> Self {
        Self(Stream::DevNull)
    }
}

impl Read for IStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Self(Stream::Fd(fd)) = self {
            let mut file = unsafe { File::from_raw_fd(*fd) };
            let res = file.read(buf);
            mem::forget(file); // omit the destruciton of the file, i.e. no call to close(fd).
            res
        } else {
            Ok(0)
        }
    }
}

impl AsRawFd for IStream {
    fn as_raw_fd(&self) -> RawFd {
        if let Self(Stream::Fd(fd)) = self {
            return *fd;
        }
        panic!("as_raw_fd() must not be called on /dev/null streams");
    }
}

pub struct OStream(Stream);

impl OStream {
    pub fn devnull() -> Self {
        Self(Stream::DevNull)
    }
}

impl Write for OStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Self(Stream::Fd(fd)) = self {
            let mut file = unsafe { File::from_raw_fd(*fd) };
            let res = file.write(buf);
            mem::forget(file); // omit the destruciton of the file, i.e. no call to close(fd).
            res
        } else {
            Ok(0)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(()) // noop
    }
}

impl AsRawFd for OStream {
    fn as_raw_fd(&self) -> RawFd {
        if let Self(Stream::Fd(fd)) = self {
            return *fd;
        }
        panic!("as_raw_fd() must not be called on /dev/null streams");
    }
}

pub fn set_stdio((ins, outs, errs): (IStream, OStream, OStream)) {
    match ins {
        IStream(Stream::Fd(fd)) => {
            dup2(fd, 0).expect("dup2(fd, STDIN_FILENO) failed");
        }
        IStream(Stream::DevNull) => {
            let fd = open_dev_null(OFlag::O_RDONLY);
            dup2(fd, 0).expect("dup2(fd, STDIN_FILENO) failed");
            close(fd).expect("close('/dev/null (RDONLY)') failed");
        }
    }

    match outs {
        OStream(Stream::Fd(fd)) => {
            dup2(fd, 1).expect("dup2(fd, STDOUT_FILENO) failed");
        }
        OStream(Stream::DevNull) => {
            let fd = open_dev_null(OFlag::O_WRONLY);
            dup2(fd, 1).expect("dup2(fd, STDOUT_FILENO) failed");
            close(fd).expect("close('/dev/null (WRONLY)') failed");
        }
    }

    match errs {
        OStream(Stream::Fd(fd)) => {
            dup2(fd, 2).expect("dup2(fd, STDERR_FILENO) failed");
        }
        OStream(Stream::DevNull) => {
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

pub struct PipeMaster {
    ins: OStream,
    outs: IStream,
    errs: IStream,
}

impl PipeMaster {
    pub fn streams(self) -> (OStream, IStream, IStream) {
        (self.ins, self.outs, self.errs)
    }
}

pub struct PipeSlave {
    ins: IStream,
    outs: OStream,
    errs: OStream,
}

impl PipeSlave {
    pub fn streams(self) -> (IStream, OStream, OStream) {
        (self.ins, self.outs, self.errs)
    }
}

pub fn create_pipes() -> (PipeMaster, PipeSlave) {
    let stdin = Pipe::new();
    let stdout = Pipe::new();
    let stderr = Pipe::new();

    let master = PipeMaster {
        ins: OStream(Stream::Fd(stdin.wr())),
        outs: IStream(Stream::Fd(stdout.rd())),
        errs: IStream(Stream::Fd(stderr.rd())),
    };
    let slave = PipeSlave {
        ins: IStream(Stream::Fd(stdin.rd())),
        outs: OStream(Stream::Fd(stdout.wr())),
        errs: OStream(Stream::Fd(stderr.wr())),
    };

    // To prevent pipe objects calling close on underlying file descriptors:
    mem::forget(stdin);
    mem::forget(stdout);
    mem::forget(stderr);

    (master, slave)
}
