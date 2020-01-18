use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::Duration;

use log::debug;
use nix::unistd::Pid;

use crate::nixtools::process::TerminationStatus;
use crate::nixtools::signal::Signalfd;
use crate::nixtools::stdio::PipeMaster;

use super::io;
use super::reactor::Reactor;
use super::signal;

pub struct Server {
    reactor: Reactor,
}

impl Server {
    pub fn new<P: AsRef<Path>>(
        container_pid: Pid,
        container_attach_path: P,
        container_logfile: P,
        container_stdio: PipeMaster,
        sigfd: Signalfd,
    ) -> Self {
        // TODO: add logger as sink to stdout & stderr scatterers

        let attach_listener = UnixListener::bind(container_attach_path).unwrap();
        attach_listener
            .set_nonblocking(true)
            .expect("Couldn't set attach listener nonblocking");

        let (ins, outs, errs) = container_stdio.streams();
        Self {
            reactor: Reactor::new(
                Duration::from_millis(5000),
                io::Gatherer::new(ins),
                io::Scatterer::new(outs),
                io::Scatterer::new(errs),
                signal::Handler::new(sigfd, container_pid),
                attach_listener,
            ),
        }
    }

    pub fn run(&mut self) -> TerminationStatus {
        debug!("[shim] serving container");
        self.reactor.run()
    }
}
