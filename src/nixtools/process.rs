use std::fmt;

use nix::errno::Errno;
use nix::sys::signal::{self, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

type ExitCode = i32;

#[derive(Copy, Clone, Debug)]
pub enum TerminationStatus {
    Exited(Pid, ExitCode),
    Signaled(Pid, Signal),
}

impl fmt::Display for TerminationStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Exited(.., code) => write!(f, "Exited with code {}", code),
            Self::Signaled(.., sig) => write!(f, "Received signal {}", sig),
        }
    }
}

impl TerminationStatus {
    pub fn pid(&self) -> Pid {
        match &self {
            Self::Exited(pid, ..) => *pid,
            Self::Signaled(pid, ..) => *pid,
        }
    }

    pub fn exit_code(&self) -> Option<ExitCode> {
        match self {
            Self::Exited(.., code) => Some(*code),
            _ => None,
        }
    }
}

pub fn get_child_termination_status() -> Option<TerminationStatus> {
    // Wait for any child state change:
    match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
        Ok(WaitStatus::Exited(pid, code)) => Some(TerminationStatus::Exited(pid, code)),

        Ok(WaitStatus::Signaled(pid, sig, ..)) => Some(TerminationStatus::Signaled(pid, sig)),

        Ok(_) => None, // non-terminal state change

        Err(nix::Error::Sys(Errno::ECHILD)) => None, // no children left

        Err(err) => panic!("waitpid() failed with error {:?}", err),
    }
}

pub enum KillResult {
    Delivered,
    ProcessNotFound,
}

pub fn kill(pid: Pid, sig: Signal) -> nix::Result<KillResult> {
    match signal::kill(pid, sig) {
        Ok(_) => Ok(KillResult::Delivered),

        Err(nix::Error::Sys(Errno::ESRCH)) => Ok(KillResult::ProcessNotFound),

        Err(err) => Err(err),
    }
}
