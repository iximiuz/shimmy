use std::os::unix::io::RawFd;

use libc::{self, c_int, c_ulong};
use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use nix::sys::signal::Signal;
use nix::unistd::setsid;
use nix::Result;

pub fn to_pipe_fd(maybe_fd: i32) -> RawFd {
    let fd = maybe_fd as c_int;

    match fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)) {
        Ok(rv) if rv != -1 => (),
        _ => panic!("fcntl(F_SETFD, FD_CLOEXEC"),
    }

    return fd;
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

pub fn _exit(status: libc::c_int) -> ! {
    unsafe {
        libc::_exit(status);
    }
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
