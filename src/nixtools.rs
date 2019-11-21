use libc::{self, c_int, c_ulong};
use nix::errno::Errno;
use nix::fcntl::{open, OFlag};
use nix::sys::signal::{sigprocmask, SigSet, SigmaskHow, Signal};
use nix::sys::stat::Mode;
use nix::unistd::{close, dup2, pipe2, setsid};
use nix::Result;
use std::os::unix::io::RawFd;

pub struct Pipe {
    rd: RawFd,
    wr: RawFd,
}

impl Pipe {
    pub fn rd(&self) -> RawFd {
        self.rd
    }
    pub fn wr(&self) -> RawFd {
        self.wr
    }
}

pub fn create_pipe() -> Pipe {
    let (rd, wr) = pipe2(OFlag::O_CLOEXEC).expect("pipe2() failed");
    Pipe { rd, wr }
}

pub fn session_start() {
    setsid().expect("sessid() failed");
}

pub fn set_child_subreaper() {
    prctl(PrctlOption::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0)
        .expect("prctl(PR_SET_CHILD_SUBREAPER) failed");
}

pub fn set_parent_death_signal(sig: Signal) {
    prctl(PrctlOption::PR_SET_PDEATHSIG, sig as c_ulong, 0, 0, 0)
        .expect("prctl(PR_SET_PDEATHSIG) failed");
}

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

pub fn signals_block(signals: &[Signal]) -> SigSet {
    let mut newmask = SigSet::empty();
    for s in signals {
        newmask.add(*s);
    }
    let mut oldmask = SigSet::empty();
    sigprocmask(SigmaskHow::SIG_BLOCK, Some(&newmask), Some(&mut oldmask))
        .expect("sigprocmask(SIG_BLOCK) failed");
    return oldmask;
}

pub fn signals_restore(mask: &SigSet) {
    sigprocmask(SigmaskHow::SIG_SETMASK, Some(&mask), None)
        .expect("sigprocmask(SIG_SETMASK) failed");
}

#[repr(i32)]
#[allow(non_camel_case_types)]
enum PrctlOption {
    PR_SET_CHILD_SUBREAPER = libc::PR_SET_CHILD_SUBREAPER,
    PR_SET_PDEATHSIG = libc::PR_SET_PDEATHSIG,
}

fn prctl(
    option: PrctlOption,
    arg2: c_ulong,
    arg3: c_ulong,
    arg4: c_ulong,
    arg5: c_ulong,
) -> Result<()> {
    let res = unsafe { libc::prctl(option as c_int, arg2, arg3, arg4, arg5) };
    Errno::result(res).map(drop)
}
