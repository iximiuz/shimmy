use nix::errno::Errno;
use nix::sys::signal::{self, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

type ExitCode = i32;

pub enum TerminationStatus {
    Exited(Pid, ExitCode),
    Signaled(Pid, Signal),
}

impl TerminationStatus {
    pub fn pid(&self) -> Pid {
        match &self {
            Self::Exited(pid, ..) => *pid,
            Self::Signaled(pid, ..) => *pid,
        }
    }

    pub fn exited_with_code(&self, expected: ExitCode) -> bool {
        match self {
            Self::Exited(.., code) if *code == expected => true,
            _ => false,
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
