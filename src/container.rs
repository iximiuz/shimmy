use std::path::Path;
use std::time::Duration;

use log::debug;
use nix::unistd::Pid;

use crate::nixtools::process::TerminationStatus;
use crate::nixtools::signal::Signalfd;
use crate::nixtools::stdio::IOStreams;

pub fn serve_container<P: AsRef<Path>>(
    sigfd: Signalfd,
    container_pid: Pid,
    container_stdio: IOStreams,
    container_logfile: P,
) -> TerminationStatus {
    debug!("[shim] serving container {}", container_pid);

    let mut server = reactor::Reactor::new(
        container_pid,
        logwriter::LogWriter::new(container_logfile),
        Duration::from_millis(5000),
    );

    server.register_signalfd(sigfd);
    server.register_container_stdout(container_stdio.Out);
    server.register_container_stderr(container_stdio.Err);

    server.run()
}

mod reactor {
    use std::time::Duration;

    use log::{debug, error, warn};
    use mio::unix::UnixReady;
    use mio::{Event, Events, Poll, PollOpt, Ready, Token};
    use nix::sys::signal::{Signal, Signal::SIGCHLD};
    use nix::unistd::Pid;

    use crate::container::logwriter::LogWriter;
    use crate::nixtools::process::{
        get_child_termination_status, kill, KillResult, TerminationStatus,
    };
    use crate::nixtools::signal::Signalfd;
    use crate::nixtools::stdio::IOStream;

    const TOKEN_STDOUT: Token = Token(10);
    const TOKEN_STDERR: Token = Token(20);
    const TOKEN_SIGNAL: Token = Token(30);

    pub struct Reactor {
        poll: Poll,
        heartbeat: Duration,
        log_writer: LogWriter,
        cont_pid: Pid,
        cont_status: Option<TerminationStatus>,
        cont_stdout: Option<IOStream>,
        cont_stderr: Option<IOStream>,
        sigfd: Option<Signalfd>,
    }

    impl Reactor {
        pub fn new(cont_pid: Pid, log_writer: LogWriter, heartbeat: Duration) -> Self {
            Self {
                poll: Poll::new().expect("mio::Poll::new() failed"),
                heartbeat: heartbeat,
                log_writer: log_writer,
                cont_pid: cont_pid,
                cont_status: None,
                cont_stdout: None,
                cont_stderr: None,
                sigfd: None,
            }
        }

        pub fn register_container_stdout(&mut self, stream: IOStream) {
            self.cont_stdout = Some(stream);
            if let Some(stream) = &self.cont_stdout {
                self.poll
                    .register(
                        stream,
                        TOKEN_STDOUT,
                        Ready::readable() | UnixReady::error() | UnixReady::hup(),
                        PollOpt::level(),
                    )
                    .expect("mio::Poll::register(container stdout) failed");
            }
        }

        pub fn register_container_stderr(&mut self, stream: IOStream) {
            self.cont_stderr = Some(stream);
            if let Some(stream) = &self.cont_stderr {
                self.poll
                    .register(
                        stream,
                        TOKEN_STDERR,
                        Ready::readable() | UnixReady::error() | UnixReady::hup(),
                        PollOpt::level(),
                    )
                    .expect("mio::Poll::register(container stderr) failed");
            }
        }

        pub fn register_signalfd(&mut self, sigfd: Signalfd) {
            self.sigfd = Some(sigfd);
            if let Some(sigfd) = &self.sigfd {
                self.poll
                    .register(
                        sigfd,
                        TOKEN_SIGNAL,
                        Ready::readable() | UnixReady::error() | UnixReady::hup(),
                        PollOpt::level(),
                    )
                    .expect("mio::Poll::register(signalfd) failed");
            }
        }

        pub fn run(&mut self) -> TerminationStatus {
            while self.cont_status.is_none() {
                if self.poll_once() == 0 {
                    debug!("[shim] still serving container {}", self.cont_pid);
                }
            }

            // Drain stdout & stderr.
            self.heartbeat = Duration::from_millis(0);
            if let Some(sigfd) = &self.sigfd {
                self.poll
                    .deregister(sigfd)
                    .expect("mio::Poll::deregister(sigfd) failed");
                self.sigfd = None;
            }

            while self.poll_once() != 0 {
                debug!("[shim] draining container IO streams");
            }

            self.cont_status.unwrap()
        }

        fn poll_once(&mut self) -> i32 {
            let mut events = Events::with_capacity(128);
            self.poll.poll(&mut events, Some(self.heartbeat)).unwrap();

            let mut event_count = 0;
            for event in events.iter() {
                event_count += 1;
                match event.token() {
                    TOKEN_STDOUT => self.handle_cont_stdout_event(&event),
                    TOKEN_STDERR => self.handle_cont_stderr_event(&event),
                    TOKEN_SIGNAL => self.handle_signalfd_event(&event),
                    _ => unreachable!(),
                }
            }
            event_count
        }

        fn handle_cont_stdout_event(&mut self, event: &Event) {
            if let Some(ref stream) = self.cont_stdout {
                if event.readiness().is_readable() {
                    let mut buf = [0; 16 * 1024];
                    match stream.read(&mut buf) {
                        Ok(0) => (),
                        Ok(nread) => self.log_writer.write_container_stdout(&buf[..nread]),
                        Err(err) => warn!("[shim] container's STDOUT errored: {}", err),
                    }
                }

                if UnixReady::from(event.readiness()).is_hup() {
                    debug!("[shim] container's STDOUT hup");
                    self.poll
                        .deregister(stream)
                        .expect("mio::Poll::deregister(container STDOUT) failed");
                    self.cont_stdout = None;
                } else if UnixReady::from(event.readiness()).is_error() {
                    warn!("[shim] container's STDOUT errored!");
                    self.poll
                        .deregister(stream)
                        .expect("mio::Poll::deregister(container STDOUT) failed");
                    self.cont_stdout = None;
                }
            }
        }

        fn handle_cont_stderr_event(&mut self, event: &Event) {
            if let Some(ref stream) = self.cont_stderr {
                if event.readiness().is_readable() {
                    let mut buf = [0; 16 * 1024];
                    match stream.read(&mut buf) {
                        Ok(0) => (),
                        Ok(nread) => self.log_writer.write_container_stderr(&buf[..nread]),
                        Err(err) => warn!("[shim] container's STDERR errored: {}", err),
                    }
                }

                if UnixReady::from(event.readiness()).is_hup() {
                    debug!("[shim] container's STDERR hup");
                    self.poll
                        .deregister(stream)
                        .expect("mio::Poll::deregister(container STDERR) failed");
                    self.cont_stderr = None;
                } else if UnixReady::from(event.readiness()).is_error() {
                    warn!("[shim] container's STDOUT errored!");
                    self.poll
                        .deregister(stream)
                        .expect("mio::Poll::deregister(container STDERR) failed");
                    self.cont_stderr = None;
                }
            }
        }

        fn handle_signalfd_event(&mut self, event: &Event) {
            if let Some(ref mut sigfd) = self.sigfd {
                if !event.readiness().is_readable() {
                    error!("Unexpected event on signalfd {:?}", event);
                    // Let it die on the following read(signalfd) attempt.
                }

                match sigfd.read_signal() {
                    SIGCHLD => self.handle_sigchld(),
                    signal => forward_signal(self.cont_pid, signal),
                }
            }
        }

        fn handle_sigchld(&mut self) {
            if let Some(status) = get_child_termination_status() {
                self.set_cont_status(status);
            }
        }

        fn set_cont_status(&mut self, status: TerminationStatus) {
            assert!(self.cont_status.is_none());
            assert!(self.cont_pid == status.pid());
            self.cont_status = Some(status);
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
}

mod logwriter {
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::path::Path;

    use chrono::Utc;
    use log::debug;

    pub struct LogWriter {
        file: File,
    }

    impl LogWriter {
        pub fn new<P: AsRef<Path>>(path: P) -> Self {
            Self {
                file: OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(path)
                    .unwrap(),
            }
        }

        pub fn write_container_stdout(&mut self, data: &[u8]) {
            debug!(
                "[shim] container's STDOUT: [{}]",
                String::from_utf8_lossy(&data)
            );
            self.write("stdout", data);
        }

        pub fn write_container_stderr(&mut self, data: &[u8]) {
            debug!(
                "[shim] container's STDERR: [{}]",
                String::from_utf8_lossy(&data)
            );
            self.write("stderr", data);
        }

        fn write(&mut self, stream: &'static str, data: &[u8]) {
            for line in data.split(|c| *c == b'\n').filter(|l| l.len() > 0) {
                write!(
                    self.file,
                    "{} {} {}\n",
                    Utc::now().to_rfc3339(),
                    stream,
                    String::from_utf8_lossy(line)
                )
                .expect("container log write failed");
            }
        }
    }
}
