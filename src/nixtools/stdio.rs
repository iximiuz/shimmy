use std::fs::File;
use std::io::{self, Read, Write};
use std::mem;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

use log::error;
use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::{close, dup2};

use crate::nixtools::pipe::Pipe;

pub struct IStream(RawFd);

impl Drop for IStream {
    fn drop(&mut self) {
        if let Err(err) = close(self.0) {
            error!("istream close({}) failed: {}", self.0, err);
        }
    }
}

impl Read for IStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut file = unsafe { File::from_raw_fd(self.0) };
        let res = file.read(buf);
        mem::forget(file); // omit the destruciton of the file, i.e. no call to close(fd).
        res
    }
}

impl AsRawFd for IStream {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

pub struct OStream(RawFd);

impl Drop for OStream {
    fn drop(&mut self) {
        if let Err(err) = close(self.0) {
            error!("ostream close({}) failed: {}", self.0, err);
        }
    }
}

impl Write for OStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut file = unsafe { File::from_raw_fd(self.0) };
        let res = file.write(buf);
        mem::forget(file); // omit the destruciton of the file, i.e. no call to close(fd).
        res
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(()) // noop
    }
}

pub fn set_stdio((ins, outs, errs): (Option<IStream>, Option<OStream>, Option<OStream>)) {
    match ins {
        Some(IStream(fd)) => {
            dup2(fd, 0).expect("dup2(fd, STDIN_FILENO) failed");
        }
        None => {
            let fd = open_dev_null(OFlag::O_RDONLY);
            dup2(fd, 0).expect("dup2(fd, STDIN_FILENO) failed");
            close(fd).expect("close('/dev/null (RDONLY)') failed");
        }
    }

    match outs {
        Some(OStream(fd)) => {
            dup2(fd, 1).expect("dup2(fd, STDOUT_FILENO) failed");
        }
        None => {
            let fd = open_dev_null(OFlag::O_WRONLY);
            dup2(fd, 1).expect("dup2(fd, STDOUT_FILENO) failed");
            close(fd).expect("close('/dev/null (WRONLY)') failed");
        }
    }

    match errs {
        Some(OStream(fd)) => {
            dup2(fd, 2).expect("dup2(fd, STDERR_FILENO) failed");
        }
        None => {
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
    ins: Option<OStream>,
    outs: Option<IStream>,
    errs: Option<IStream>,
}

impl PipeMaster {
    pub fn streams(self) -> (Option<OStream>, Option<IStream>, Option<IStream>) {
        (self.ins, self.outs, self.errs)
    }
}

pub struct PipeSlave {
    ins: Option<IStream>,
    outs: Option<OStream>,
    errs: Option<OStream>,
}

impl PipeSlave {
    pub fn streams(self) -> (Option<IStream>, Option<OStream>, Option<OStream>) {
        (self.ins, self.outs, self.errs)
    }
}

pub fn create_pipes(
    use_stdin: bool,
    use_stdout: bool,
    use_stderr: bool,
) -> (PipeMaster, PipeSlave) {
    let mut master = PipeMaster {
        ins: None,
        outs: None,
        errs: None,
    };
    let mut slave = PipeSlave {
        ins: None,
        outs: None,
        errs: None,
    };

    if use_stdin {
        let stdin = Pipe::new();
        master.ins = Some(OStream(stdin.wr()));
        slave.ins = Some(IStream(stdin.rd()));

        // To prevent pipe objects calling close on underlying file descriptors:
        mem::forget(stdin);
    }
    if use_stdout {
        let stdout = Pipe::new();
        master.outs = Some(IStream(stdout.rd()));
        slave.outs = Some(OStream(stdout.wr()));

        // To prevent pipe objects calling close on underlying file descriptors:
        mem::forget(stdout);
    }
    if use_stderr {
        let stderr = Pipe::new();
        master.errs = Some(IStream(stderr.rd()));
        slave.errs = Some(OStream(stderr.wr()));

        // To prevent pipe objects calling close on underlying file descriptors:
        mem::forget(stderr);
    }

    (master, slave)
}
