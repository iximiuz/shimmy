use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;
use std::time::Duration;

use log::{debug, error, warn};
use mio::unix::{EventedFd, UnixReady};
use mio::{Event, Events, Poll, PollOpt, Ready, Token};

use super::io;
use super::signal;
use crate::nixtools::process::TerminationStatus;

const TOKEN_STDOUT: Token = Token(10);
const TOKEN_STDERR: Token = Token(20);
const TOKEN_SIGNAL: Token = Token(30);
const TOKEN_ATTACH: Token = Token(40);
const TOKEN_UNUSED: Token = Token(1000);

pub struct Reactor {
    poll: Poll,
    heartbeat: Duration,
    stdin_gatherer: Option<io::Gatherer>,
    stdin_once: bool,
    stdout_scatterer: Option<io::Scatterer>,
    stderr_scatterer: Option<io::Scatterer>,
    signal_handler: signal::Handler,
    attach_listener: UnixListener,
    attach_streams: HashMap<Token, RawFd>,
    attach_last_token: Token,
}

impl Reactor {
    pub fn new(
        heartbeat: Duration,
        stdin_gatherer: Option<io::Gatherer>,
        stdin_once: bool,
        stdout_scatterer: Option<io::Scatterer>,
        stderr_scatterer: Option<io::Scatterer>,
        signal_handler: signal::Handler,
        attach_listener: UnixListener,
    ) -> Self {
        let poll = Poll::new().expect("mio::Poll::new() failed");

        if let Some(scatterer) = stdout_scatterer.as_ref() {
            poll.register(
                scatterer,
                TOKEN_STDOUT,
                Ready::readable() | UnixReady::hup(),
                PollOpt::level(),
            )
            .expect("mio::Poll::register(container stdout) failed");
        }

        if let Some(scatterer) = stderr_scatterer.as_ref() {
            poll.register(
                scatterer,
                TOKEN_STDERR,
                Ready::readable() | UnixReady::hup(),
                PollOpt::level(),
            )
            .expect("mio::Poll::register(container stderr) failed");
        }

        poll.register(
            &signal_handler,
            TOKEN_SIGNAL,
            Ready::readable() | UnixReady::error(),
            PollOpt::level(),
        )
        .expect("mio::Poll::register(signalfd) failed");

        poll.register(
            &EventedFd(&attach_listener.as_raw_fd()),
            TOKEN_ATTACH,
            Ready::readable() | UnixReady::error(),
            PollOpt::level(),
        )
        .expect("mio::Poll::register(attach listener) failed");

        Self {
            poll: poll,
            heartbeat: heartbeat,
            stdin_gatherer: stdin_gatherer,
            stdin_once: stdin_once,
            stdout_scatterer: stdout_scatterer,
            stderr_scatterer: stderr_scatterer,
            signal_handler: signal_handler,
            attach_listener: attach_listener,
            attach_streams: HashMap::new(),
            attach_last_token: TOKEN_UNUSED,
        }
    }

    pub fn run(&mut self) -> TerminationStatus {
        while self.signal_handler.container_status().is_none() {
            if self.poll_once() == 0 {
                debug!("[shim] still serving container");
            }
        }

        // Drain stdout & stderr.
        self.poll
            .deregister(&self.signal_handler)
            .expect("mio::Poll::deregister(signalfd) failed");
        self.poll
            .deregister(&EventedFd(&self.attach_listener.as_raw_fd()))
            .expect("mio::Poll::deregister(attach listener) failed");
        self.heartbeat = Duration::from_millis(0);

        while self.poll_once() != 0 {
            debug!("[shim] draining container IO streams");
        }

        self.signal_handler.container_status().unwrap()
    }

    fn poll_once(&mut self) -> i32 {
        let mut events = Events::with_capacity(128);
        self.poll
            .poll(&mut events, Some(self.heartbeat))
            .expect("mio::Poll::poll() failed");

        let mut event_count = 0;
        for event in events.iter() {
            event_count += 1;
            match event.token() {
                TOKEN_STDOUT => self.handle_stdout_event(event),
                TOKEN_STDERR => self.handle_stderr_event(event),
                TOKEN_SIGNAL => self.signal_handler.handle_signal(),
                TOKEN_ATTACH => self.handle_attach_listener_event(event),
                _ => self.handle_attach_stream_event(event),
            }
        }
        event_count
    }

    fn handle_stdout_event(&mut self, event: Event) {
        if self.stdout_scatterer.is_none() {
            warn!("[shim] dubious, got event on already closed STDOUT");
            return;
        }

        if event.readiness().is_readable() {
            match self.stdout_scatterer.as_mut().unwrap().scatter() {
                Ok(nbytes) => {
                    debug!(
                        "[shim] scattered {} byte(s) from container's STDOUT",
                        nbytes
                    );
                    if nbytes == 0 {
                        self.deregister_stdout_scatterer();
                    }
                }
                Err(err) => error!("[shim] failed scattering container's STDOUT: {:?}", err),
            }
        } else if UnixReady::from(event.readiness()).is_hup() {
            debug!("[shim] STDOUT HUP");
            self.deregister_stdout_scatterer();
        }
    }

    fn handle_stderr_event(&mut self, event: Event) {
        if self.stderr_scatterer.is_none() {
            warn!("[shim] dubious, got event on already closed STDERR");
            return;
        }

        if event.readiness().is_readable() {
            match self.stderr_scatterer.as_mut().unwrap().scatter() {
                Ok(nbytes) => {
                    debug!(
                        "[shim] scattered {} byte(s) from container's STDERR",
                        nbytes
                    );
                    if nbytes == 0 {
                        self.deregister_stderr_scatterer();
                    }
                }
                Err(err) => error!("[shim] failed scattering container's STDERR: {:?}", err),
            }
        } else if UnixReady::from(event.readiness()).is_hup() {
            debug!("[shim] STDERR HUP");
            self.deregister_stderr_scatterer();
        }
    }

    fn handle_attach_listener_event(&mut self, event: Event) {
        if UnixReady::from(event.readiness()).is_error() {
            match self.attach_listener.take_error() {
                Ok(None) => error!("[shim] attach listener event with error flag"),
                Ok(Some(err)) => error!("[shim] attach listener error: {}", err),
                Err(err) => error!("[shim] attach listener take_error() failed: {}", err),
            }
            return;
        }

        match self.attach_listener.accept() {
            Ok((stream, _)) => {
                debug!("[shim] new attach socket stream");
                let token = self.register_attach_stream(stream.as_raw_fd());
                let stream_rc: Rc<RefCell<UnixStream>> = Rc::new(RefCell::new(stream));
                if let Some(ref mut stdin_gatherer) = self.stdin_gatherer {
                    stdin_gatherer.add_source(token, stream_rc.clone());
                }
                if let Some(ref mut stdout_scatterer) = self.stdout_scatterer {
                    stdout_scatterer.add_sink(stream_rc.clone());
                }
                if let Some(ref mut stderr_scatterer) = self.stderr_scatterer {
                    stderr_scatterer.add_sink(stream_rc.clone());
                }
            }
            Err(err) => error!("[shim] attach listener accept failed: {}", err),
        }
    }

    fn handle_attach_stream_event(&mut self, event: Event) {
        if self.stdin_gatherer.is_none() {
            warn!("[shim] container's STDIN has been already closed");
            return;
        }

        let stdin_gatherer = self.stdin_gatherer.as_mut().unwrap();
        if event.readiness().is_readable() {
            match stdin_gatherer.gather(event.token()) {
                Ok(nbytes) => {
                    debug!("[shim] gathered {} byte(s) to container's STDIN", nbytes);
                    if nbytes == 0 {
                        debug!("[shim] attach socket stream eof");
                        stdin_gatherer.remove_source(event.token());
                        self.deregister_attach_stream(event.token());
                        if self.stdin_once {
                            self.stdin_gatherer = None;
                        }
                    }
                }
                Err(io::Error::Source(err)) => {
                    error!("[shim] attach socket stream read error: {}", err);
                    stdin_gatherer.remove_source(event.token());
                    self.deregister_attach_stream(event.token());
                    if self.stdin_once {
                        self.stdin_gatherer = None;
                    }
                }
                Err(io::Error::Sink(err)) => {
                    error!("[shim] write to container's STDIN failed: {}", err);
                }
            }
        } else if UnixReady::from(event.readiness()).is_hup() {
            debug!("[shim] attach socket stream HUP");
            stdin_gatherer.remove_source(event.token());
            self.deregister_attach_stream(event.token());
        }
    }

    fn deregister_stdout_scatterer(&mut self) {
        self.poll
            .deregister(self.stdout_scatterer.as_ref().unwrap())
            .expect("mio::Poll::deregister(container STDOUT) failed");
        self.stdout_scatterer = None;
    }

    fn deregister_stderr_scatterer(&mut self) {
        self.poll
            .deregister(self.stderr_scatterer.as_ref().unwrap())
            .expect("mio::Poll::deregister(container STDERR) failed");
        self.stderr_scatterer = None;
    }

    fn register_attach_stream(&mut self, fd: RawFd) -> Token {
        self.attach_last_token = Token(usize::from(self.attach_last_token) + 1);

        self.poll
            .register(
                &EventedFd(&fd),
                self.attach_last_token,
                Ready::readable() | UnixReady::error() | UnixReady::hup(),
                PollOpt::level(),
            )
            .expect("mio::Poll::register(attach stream) failed");

        self.attach_streams.insert(self.attach_last_token, fd);

        self.attach_last_token
    }

    fn deregister_attach_stream(&mut self, token: Token) {
        if let Some(fd) = self.attach_streams.remove(&token) {
            self.poll
                .deregister(&EventedFd(&fd))
                .expect("mio::Poll::deregister(attach conn) failed");
        } else {
            warn!("[shim] attach stream with token {:?} not found", token);
        }
    }
}
