use std::path::Path;
use std::time::Duration;

use log::debug;
use nix::unistd::Pid;

use crate::attach::Listener as AttachListener;
use crate::nixtools::process::TerminationStatus;
use crate::nixtools::signal::Signalfd;
use crate::nixtools::stdio::IOStreams;

pub fn serve_container<P: AsRef<Path>>(
    sigfd: Signalfd,
    attach_listener: AttachListener,
    container_pid: Pid,
    container_stdio: IOStreams,
    container_logfile: P,
) -> TerminationStatus {
    debug!("[shim] serving container {}", container_pid);

    let mut server = reactor::Reactor::new(
        container_pid,
        Duration::from_millis(5000),
        logwriter::LogWriter::new(container_logfile),
    );

    server.register_container_stdout(container_stdio.Out);
    server.register_container_stderr(container_stdio.Err);
    server.register_signalfd(sigfd);
    server.register_attach_server(reactor::AttachServer::new(attach_listener));

    server.run()
}

mod reactor {
    use std::collections::HashMap;
    use std::io;
    use std::time::Duration;

    use log::{debug, error, warn};
    use mio::unix::UnixReady;
    use mio::{Event, Events, Poll, PollOpt, Ready, Token};
    use nix::sys::signal::{Signal, Signal::SIGCHLD};
    use nix::unistd::Pid;

    use crate::attach::{Connection as AttachConnection, Listener as AttachListener};
    use crate::container::logwriter::LogWriter;
    use crate::nixtools::process::{
        get_child_termination_status, kill, KillResult, TerminationStatus,
    };
    use crate::nixtools::signal::Signalfd;
    use crate::nixtools::stdio::IOStream;

    const TOKEN_STDOUT: Token = Token(10);
    const TOKEN_STDERR: Token = Token(20);
    const TOKEN_SIGNAL: Token = Token(30);
    const TOKEN_ATTACH: Token = Token(40);

    pub struct AttachServer {
        listener: AttachListener,
        connections: HashMap<Token, AttachConnection>,
        next_conn_index: usize,
    }

    impl AttachServer {
        pub fn new(listener: AttachListener) -> Self {
            Self {
                listener: listener,
                connections: HashMap::new(),
                next_conn_index: usize::from(TOKEN_ATTACH) + 1,
            }
        }

        fn accept(&mut self) -> io::Result<(&AttachConnection, Token)> {
            let (sock, _) = self.listener.accept()?;

            let token = Token(self.next_conn_index);
            self.next_conn_index += 1;

            self.connections.insert(token, AttachConnection::new(sock));
            Ok((self.connections.get(&token).unwrap(), token))
        }
    }

    pub struct Reactor {
        poll: Poll,
        cont_pid: Pid,
        heartbeat: Duration,
        log_writer: LogWriter,
        cont_status: Option<TerminationStatus>,
        cont_stdout: Option<IOStream>,
        cont_stderr: Option<IOStream>,
        sigfd: Option<Signalfd>,
        attach_server: Option<AttachServer>,
    }

    impl Reactor {
        pub fn new(cont_pid: Pid, heartbeat: Duration, log_writer: LogWriter) -> Self {
            Self {
                poll: Poll::new().expect("mio::Poll::new() failed"),
                cont_pid: cont_pid,
                heartbeat: heartbeat,
                log_writer: log_writer,
                cont_status: None,
                cont_stdout: None,
                cont_stderr: None,
                sigfd: None,
                attach_server: None,
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

        pub fn register_attach_server(&mut self, server: AttachServer) {
            self.attach_server = Some(server);
            if let Some(server) = &self.attach_server {
                self.poll
                    .register(
                        &server.listener,
                        TOKEN_ATTACH,
                        Ready::readable() | UnixReady::error(),
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
                    TOKEN_ATTACH => self.handle_attach_listener_event(&event),
                    _ => self.handle_attach_conn_event(&event),
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
                assert!(self.cont_status.is_none());
                assert!(self.cont_pid == status.pid());
                self.cont_status = Some(status);
            }
        }

        fn handle_attach_listener_event(&mut self, event: &Event) {
            if let Some(ref mut attach_server) = self.attach_server {
                if !event.readiness().is_readable() {
                    error!(
                        "[shim] attach listener errored: {:?}",
                        attach_server.listener.take_error()
                    );
                    return;
                }
                match attach_server.accept() {
                    Ok((conn, token)) => {
                        debug!("[shim] new attach socket conn");
                        self.poll
                            .register(
                                conn,
                                token,
                                Ready::readable()
                                    | Ready::writable()
                                    | UnixReady::error()
                                    | UnixReady::hup(),
                                PollOpt::level(),
                            )
                            .expect("mio::Poll::register(signalfd) failed");
                    }
                    Err(err) => error!("[shim] attach server accept failed: {}", err),
                }
            }
        }

        fn handle_attach_conn_event(&mut self, event: &Event) {
            if let Some(ref mut attach_server) = self.attach_server {
                if event.readiness().is_readable() {
                    let conn = attach_server.connections.get_mut(&event.token()).unwrap();
                    let buf = conn.read();
                    debug!("READ FROM ATTACH SOCK: {}", buf);
                }
            }
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
