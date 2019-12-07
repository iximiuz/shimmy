use std::fmt;

use log::debug;
use nix::sys::signal::{
    Signal,
    Signal::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM},
};
use nix::unistd::Pid;

use crate::nixtools::process::{
    get_child_termination_status, kill, KillResult, TerminationStatus as ProcessTerminationStatus,
};
use crate::nixtools::signal::Signalfd;

#[derive(Copy, Clone, Debug)]
pub enum TerminationStatus {
    // (runtime_status, inflight_signal)
    Solitary(ProcessTerminationStatus, Option<Signal>),

    // (runtime_status, container_status, inflight_signal)
    Conjoint(
        ProcessTerminationStatus,
        ProcessTerminationStatus,
        Option<Signal>,
    ),
}

impl fmt::Display for TerminationStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Solitary(rt, None) => write!(f, "Runtime {}.", rt),
            Self::Solitary(rt, Some(sig)) => {
                write!(f, "Runtime {}. Beware: inflight {} detected.", rt, sig)
            }
            Self::Conjoint(rt, ct, None) => write!(f, "Runtime {}. Container {}.", rt, ct),
            Self::Conjoint(rt, ct, Some(sig)) => write!(
                f,
                "Runtime {}. Container {}. Beware: inflight {} detected.",
                rt, ct, sig
            ),
        }
    }
}

impl TerminationStatus {
    fn new(
        runtime: ProcessTerminationStatus,
        container: Option<ProcessTerminationStatus>,
        inflight: Option<Signal>,
    ) -> Self {
        match container {
            None => Self::Solitary(runtime, inflight),
            Some(container) => Self::Conjoint(runtime, container, inflight),
        }
    }
}

pub fn await_runtime_termination(sigfd: &mut Signalfd, runtime_pid: Pid) -> TerminationStatus {
    debug!("[shim] awaiting runtime termination...");

    let mut container: Option<ProcessTerminationStatus> = None;
    let mut inflight: Option<Signal> = None;
    loop {
        match sigfd.read_signal() {
            SIGCHLD => {
                debug!("[shim] SIGCHLD received, querying runtime status");

                match get_termination_statuses(runtime_pid) {
                    (Some(rt), ct) => {
                        assert!(
                            ct.is_none() || container.is_none(),
                            "ambiguous container termination status"
                        );
                        return TerminationStatus::new(rt, container.or(ct), inflight);
                    }

                    (None, Some(ct)) => {
                        assert!(
                            container.is_none(),
                            "ambiguous container termination status"
                        );
                        container = Some(ct); // Keep it for later use.
                    }

                    (None, None) => (), // Continue...
                }
            }

            sig if [SIGINT, SIGQUIT, SIGTERM].contains(&sig) => {
                debug!("[shim] {} received, propagating to runtime", sig);

                match kill(runtime_pid, sig) {
                    Ok(KillResult::Delivered) => (), // Keep waiting for runtime termination...

                    Ok(KillResult::ProcessNotFound) => {
                        // Runtime already has exited, keep the signal
                        // to send it to the container once we know its PID.
                        inflight = Some(sig);
                    }

                    Err(err) => panic!("kill(runtime_pid, {}) failed: {:?}", sig, err),
                }
            }
            sig => panic!("unexpected signal received {:?}", sig),
        };
    }
}

fn get_termination_statuses(
    runtime_pid: Pid,
) -> (
    Option<ProcessTerminationStatus>,
    Option<ProcessTerminationStatus>,
) {
    let mut runtime: Option<ProcessTerminationStatus> = None;
    let mut container: Option<ProcessTerminationStatus> = None;

    while let Some(status) = get_child_termination_status() {
        if status.pid() == runtime_pid {
            assert!(runtime.is_none());
            runtime = Some(status);
        } else {
            assert!(container.is_none());
            container = Some(status);
        }
    }

    (runtime, container)
}
