use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::Duration;

use log::{debug, error, warn};
use mio::unix::{EventedFd, UnixReady};
use mio::{Events, Poll, PollOpt, Ready, Token};
use nix::sys::signal::{Signal, Signal::SIGCHLD};
use nix::unistd::Pid;

use crate::nixtools::process::{get_child_termination_status, kill, KillResult, TerminationStatus};
use crate::nixtools::signal::Signalfd;
use crate::nixtools::stdio::IOStreams;

pub fn serve_container<P: AsRef<Path>>(
    mut sigfd: Signalfd,
    container_pid: Pid,
    container_stdio: IOStreams,
    _container_logfile: P,
) -> TerminationStatus {
    debug!("[shim] serving container {}", container_pid);

    let poll = Poll::new().expect("mio::Poll::new() failed");
    poll.register(
        &EventedFd(&sigfd.as_raw_fd()),
        Token(10),
        Ready::readable() | UnixReady::error() | UnixReady::hup(),
        PollOpt::level(),
    )
    .expect("mio::Poll::register(signalfd) failed");

    poll.register(
        &container_stdio.Out,
        Token(20),
        Ready::readable() | UnixReady::error() | UnixReady::hup(),
        PollOpt::level(),
    )
    .expect("mio::Poll::register(container stdout) failed");

    poll.register(
        &container_stdio.Err,
        Token(30),
        Ready::readable() | UnixReady::error() | UnixReady::hup(),
        PollOpt::level(),
    )
    .expect("mio::Poll::register(container stderr) failed");

    let mut events = Events::with_capacity(1024);
    'outer: loop {
        events.clear();
        poll.poll(&mut events, Some(Duration::from_millis(5000)))
            .unwrap();

        let mut triggered = 0;

        for event in events.iter() {
            triggered += 1;

            match event.token() {
                Token(10) => {
                    if !event.readiness().is_readable() {
                        error!("Unexpected event on signalfd {:?}", event);
                        // We let it die on the following read(signalfd) attempt.
                    }

                    let sig = sigfd.read_signal();
                    if sig == SIGCHLD {
                        break 'outer;
                    } else {
                        forward_signal(container_pid, sig);
                    }
                }

                Token(20) => {
                    if !event.readiness().is_readable() {
                        warn!(
                            "[shim] non-readable event {:?} on container's STDOUT",
                            event
                        );
                        poll.deregister(&container_stdio.Out)
                            .expect("mio::Poll::deregister(container STDOUT) failed");
                    } else {
                        let mut buf = [0; 16 * 1024];
                        let nread = container_stdio.Out.read(&mut buf);
                        debug!(
                            "[shim] container STDOUT [{}]: {}",
                            nread,
                            String::from_utf8_lossy(&buf)
                        );
                    }
                }

                Token(30) => {}

                _ => unreachable!(),
            }
        }

        if triggered == 0 {
            debug!("[shim] still serving container {}", container_pid);
        }
    }

    // TODO: drain master stdio?

    get_child_termination_status().unwrap()
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
