use std::cell::RefCell;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use log::debug;
use nix::unistd::Pid;

use crate::nixtools::process::TerminationStatus;
use crate::nixtools::signal::Signalfd;
use crate::nixtools::stdio::PipeMaster;

use super::io;
use super::logger::{Logger, Writer};
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
        let attach_listener = UnixListener::bind(container_attach_path).unwrap();
        attach_listener
            .set_nonblocking(true)
            .expect("Couldn't set attach listener nonblocking");

        let logger = Rc::new(RefCell::new(Logger::new(container_logfile)));

        let (stdin, stdout, stderr) = container_stdio.streams();

        let mut scatterer_stdout = io::Scatterer::new(stdout);
        scatterer_stdout.add_sink(Rc::new(RefCell::new(Writer::stdout(logger.clone()))));

        let mut scatterer_stderr = io::Scatterer::new(stderr);
        scatterer_stderr.add_sink(Rc::new(RefCell::new(Writer::stderr(logger.clone()))));

        Self {
            reactor: Reactor::new(
                Duration::from_millis(5000),
                io::Gatherer::new(stdin),
                scatterer_stdout,
                scatterer_stderr,
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
