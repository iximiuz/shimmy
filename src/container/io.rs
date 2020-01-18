use std::io::Result;
use std::os::unix::io::AsRawFd;

use mio::{event::Evented, unix::EventedFd, Poll, PollOpt, Ready, Token};

use crate::nixtools::stdio::{IStream, OStream};

pub enum Status {
    Forwarded(i32, bool),
}

pub struct Gatherer {
    sink: OStream,
}

impl Gatherer {
    pub fn new(sink: OStream) -> Self {
        Self { sink: sink }
    }

    pub fn gather(&self) -> Result<Status> {
        Ok(Status::Forwarded(0, true))
    }
}

impl Evented for Gatherer {
    fn register(&self, poll: &Poll, token: Token, interest: Ready, opts: PollOpt) -> Result<()> {
        EventedFd(&self.sink.as_raw_fd()).register(poll, token, interest, opts)
    }

    fn reregister(&self, poll: &Poll, token: Token, interest: Ready, opts: PollOpt) -> Result<()> {
        EventedFd(&self.sink.as_raw_fd()).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> Result<()> {
        EventedFd(&self.sink.as_raw_fd()).deregister(poll)
    }
}

pub struct Scatterer {
    source: IStream,
}

impl Scatterer {
    pub fn new(source: IStream) -> Self {
        Self { source: source }
    }

    pub fn scatter(&self) -> Result<Status> {
        Ok(Status::Forwarded(0, true))
    }

    //     let mut buf = [0; 16 * 1024];
    //     match stream.read(&mut buf) {
    //         Ok(0) => (),
    //         Ok(nread) => self.log_writer.write_container_stdout(&buf[..nread]),
    //         Err(err) => warn!("[shim] container's STDOUT errored: {}", err),
    //     }
}

impl Evented for Scatterer {
    fn register(&self, poll: &Poll, token: Token, interest: Ready, opts: PollOpt) -> Result<()> {
        EventedFd(&self.source.as_raw_fd()).register(poll, token, interest, opts)
    }

    fn reregister(&self, poll: &Poll, token: Token, interest: Ready, opts: PollOpt) -> Result<()> {
        EventedFd(&self.source.as_raw_fd()).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> Result<()> {
        EventedFd(&self.source.as_raw_fd()).deregister(poll)
    }
}

// impl Evented for IOStream {
//     fn register(
//         &self,
//         poll: &Poll,
//         token: Token,
//         interest: Ready,
//         opts: PollOpt,
//     ) -> io::Result<()> {
//         if let Self::Fd(fd) = self {
//             EventedFd(fd).register(poll, token, interest, opts)
//         } else {
//             panic!("not implemented!");
//         }
//     }
//
//     fn reregister(
//         &self,
//         poll: &Poll,
//         token: Token,
//         interest: Ready,
//         opts: PollOpt,
//     ) -> io::Result<()> {
//         if let Self::Fd(fd) = self {
//             EventedFd(fd).reregister(poll, token, interest, opts)
//         } else {
//             panic!("not implemented!");
//         }
//     }
//
//     fn deregister(&self, poll: &Poll) -> io::Result<()> {
//         if let Self::Fd(fd) = self {
//             EventedFd(fd).deregister(poll)
//         } else {
//             panic!("not implemented!");
//         }
//     }
// }
//
