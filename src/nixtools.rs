use libc::{self, c_int, c_ulong};
use nix::errno::Errno;
use nix::sys::signal::Signal;
use nix::unistd::{self};
use nix::Result;

pub fn set_child_subreaper() {
    prctl(PrctlOption::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0).expect("oops");
}

pub fn set_parent_death_signal(sig: Signal) {
    prctl(PrctlOption::PR_SET_PDEATHSIG, sig as c_ulong, 0, 0, 0).expect("oops");
}

pub fn setsid() {
    unistd::setsid().expect("must always succeed, see man 2 sessid");
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

