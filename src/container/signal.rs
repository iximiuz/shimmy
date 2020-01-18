use std::io;

use log::{debug, warn};
use mio::{Evented, Poll, PollOpt, Ready, Token};
use nix::sys::signal::{Signal, Signal::SIGCHLD};
use nix::unistd::Pid;

use crate::nixtools::process::{get_child_termination_status, kill, KillResult, TerminationStatus};
use crate::nixtools::signal::Signalfd;

pub struct Handler {
    sigfd: Signalfd,
    container_pid: Pid,
    container_status: Option<TerminationStatus>,
}

impl Handler {
    pub fn new(sigfd: Signalfd, container_pid: Pid) -> Self {
        Self {
            sigfd: sigfd,
            container_pid: container_pid,
            container_status: None,
        }
    }

    pub fn container_status(&self) -> Option<TerminationStatus> {
        self.container_status
    }

    pub fn handle_signal(&mut self) {
        match self.sigfd.read_signal() {
            SIGCHLD => self.handle_sigchld(),
            signal => forward_signal(self.container_pid, signal),
        }
    }

    fn handle_sigchld(&mut self) {
        if let Some(status) = get_child_termination_status() {
            assert!(self.container_pid == status.pid());
            assert!(self.container_status.is_none());
            self.container_status = Some(status);
        }
    }
}

impl Evented for Handler {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.sigfd.register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.sigfd.reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        self.sigfd.deregister(poll)
    }
}

fn forward_signal(container_pid: Pid, signal: Signal) {
    debug!(
        "[shimmy] forwarding signal {} to container {}",
        signal, container_pid
    );

    match kill(container_pid, signal) {
        Ok(KillResult::Delivered) => (),
        Ok(KillResult::ProcessNotFound) => {
            warn!("[shim] failed to forward signal to container, probably exited")
        }
        Err(err) => warn!("[shim] failed to forward signal to container: {}", err),
    }
}
