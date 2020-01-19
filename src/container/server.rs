use std::cell::RefCell;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use log::debug;
use nix::unistd::Pid;

use crate::nixtools::process::TerminationStatus;
use crate::nixtools::signal::Signalfd;
use crate::nixtools::stdio::{IStream, OStream};

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
        container_attachfile: P,
        container_logfile: P,
        (container_stdin, container_stdout, container_stderr): (
            Option<OStream>,
            Option<IStream>,
            Option<IStream>,
        ),
        stdin_once: bool,
        sigfd: Signalfd,
    ) -> Self {
        let attach_listener = UnixListener::bind(container_attachfile).unwrap();
        attach_listener
            .set_nonblocking(true)
            .expect("Couldn't set attach listener nonblocking");

        let logger = Rc::new(RefCell::new(Logger::new(container_logfile)));

        let stdin_gatherer = match container_stdin {
            Some(stream) => Some(io::Gatherer::new(stream)),
            None => None,
        };

        let stdout_scatterer = match container_stdout {
            Some(stream) => {
                let mut scatterer = io::Scatterer::stdout(stream);
                scatterer.add_sink(Rc::new(RefCell::new(Writer::stdout(logger.clone()))));
                Some(scatterer)
            }
            None => None,
        };

        let stderr_scatterer = match container_stderr {
            Some(stream) => {
                let mut scatterer = io::Scatterer::stderr(stream);
                scatterer.add_sink(Rc::new(RefCell::new(Writer::stderr(logger.clone()))));
                Some(scatterer)
            }
            None => None,
        };

        Self {
            reactor: Reactor::new(
                Duration::from_millis(5000),
                stdin_gatherer,
                stdin_once,
                stdout_scatterer,
                stderr_scatterer,
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
