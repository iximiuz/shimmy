use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Result, Write};
use std::os::unix::io::AsRawFd;
use std::rc::Rc;

use mio::{event::Evented, unix::EventedFd, Poll, PollOpt, Ready, Token};

use crate::nixtools::stdio::{IStream, OStream};

pub enum Status {
    // tuple (number of read bytes, met eof)
    Forwarded(usize, bool),
}

pub struct Gatherer {
    sink: OStream,
    sources: HashMap<Token, Rc<RefCell<dyn Read>>>,
}

impl Gatherer {
    pub fn new(sink: OStream) -> Self {
        Self {
            sink: sink,
            sources: HashMap::new(),
        }
    }

    pub fn gather(&self, _token: Token) -> Result<Status> {
        // TODO: implement me!
        Ok(Status::Forwarded(0, true))
    }

    pub fn add_source(&mut self, token: Token, source: Rc<RefCell<dyn Read>>) {
        self.sources.insert(token, source);
    }

    pub fn remove_source(&mut self, token: Token) {
        self.sources.remove(&token);
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
    sinks: HashMap<usize, Rc<RefCell<dyn Write>>>,
    next_sink_seq_no: usize,
}

impl Scatterer {
    pub fn new(source: IStream) -> Self {
        Self {
            source: source,
            sinks: HashMap::new(),
            next_sink_seq_no: 0,
        }
    }

    pub fn scatter(&mut self) -> Result<Status> {
        let mut buf = [0; 16 * 1024];
        let nread = self.source.read(&mut buf)?;
        if nread > 0 {
            self.sinks
                .retain(|_, writer| writer.borrow_mut().write(&buf[..nread]).is_ok());
        }
        Ok(Status::Forwarded(nread, nread == 0))
    }

    pub fn add_sink(&mut self, sink: Rc<RefCell<dyn Write>>) {
        self.sinks.insert(self.next_sink_seq_no, sink);
        self.next_sink_seq_no += 1;
    }
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
