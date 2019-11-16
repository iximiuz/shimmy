use libc::{self, c_int, c_ulong};
use nix::errno::Errno;
use nix::fcntl::{open, OFlag};
use nix::sys::signal::{sigprocmask, SigSet, SigmaskHow, Signal};
use nix::sys::stat::Mode;
use nix::unistd::{close, dup2, pipe2, setsid};
use nix::Result;
use std::os::unix::io::RawFd;

pub fn null_stdio_streams() {
    let dev_null_r = open(
        "/dev/null",
        OFlag::O_RDONLY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .expect("open('/dev/null') for reading failed");

    let dev_null_w = open(
        "/dev/null",
        OFlag::O_WRONLY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .expect("open('/dev/null') for writing failed");

    set_stdio_streams(STDIO {
        IN: Some(dev_null_r),
        OUT: Some(dev_null_w),
        ERR: Some(dev_null_w),
    });

    close(dev_null_r).expect("close('/dev/null (RD)') failed");
    close(dev_null_w).expect("close('/dev/null (WR)') failed");
}

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
    Pipe{ rd, wr }
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

pub struct STDIO {
    pub IN: Option<RawFd>,
    pub OUT: Option<RawFd>,
    pub ERR: Option<RawFd>,
}

pub fn set_stdio_streams(streams: STDIO) {
    if let Some(fd) = streams.IN {
        dup2(fd, 0).expect("dup2(fd, STDIN_FILENO) failed");
    }
    if let Some(fd) = streams.OUT {
        dup2(fd, 1).expect("dup2(fd, STDOUT_FILENO) failed");
    }
    if let Some(fd) = streams.ERR {
        dup2(fd, 2).expect("dup2(fd, STDERR_FILENO) failed");
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
