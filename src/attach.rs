use std::io::{self, Error, Read, Result};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::path::Path;

use mio::event::Evented;
use mio::unix::EventedFd;
use mio::{Poll, PollOpt, Ready, Token};

pub struct Listener(UnixListener);

impl Listener {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let sock = UnixListener::bind(path).unwrap();
        sock.set_nonblocking(true)
            .expect("Couldn't set non blocking");
        Self(sock)
    }

    pub fn accept(&self) -> io::Result<(UnixStream, SocketAddr)> {
        self.0.accept()
    }

    pub fn take_error(&self) -> Result<Option<Error>> {
        self.0.take_error()
    }
}

impl Evented for Listener {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.0.as_raw_fd()).register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.0.as_raw_fd()).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        EventedFd(&self.0.as_raw_fd()).deregister(poll)
    }
}

pub struct Connection {
    sock: UnixStream,
    // TODO: add readable: bool
    // TODO: add writable: bool
}

impl Connection {
    pub fn new(sock: UnixStream) -> Self {
        Self { sock: sock }
    }

    pub fn read(&mut self) -> String {
        let mut buf = String::new();
        self.sock.read_to_string(&mut buf).unwrap();
        buf
    }
}

impl Evented for Connection {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.sock.as_raw_fd()).register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.sock.as_raw_fd()).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        EventedFd(&self.sock.as_raw_fd()).deregister(poll)
    }
}
