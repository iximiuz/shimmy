use nix::sys::signal::Signal::SIGCHLD;
use nix::unistd::Pid;

use crate::nixtools::process::{get_child_termination_status, TerminationStatus};
use crate::nixtools::signal::Signalfd;
use crate::nixtools::stdio::IOStreams;

pub fn serve_container(
    sigfd: &mut Signalfd,
    container_pid: Pid,
    _container_stdio: IOStreams,
) -> TerminationStatus {
    // mio->run();
    // drain master stdio
    let sig = sigfd.read_signal();
    assert!(sig == SIGCHLD);
    TerminationStatus::Exited(container_pid, 0);

    get_child_termination_status().unwrap()
}
