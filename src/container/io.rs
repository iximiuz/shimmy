use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;
use std::rc::Rc;
use std::result;

use log::warn;
use mio::{event::Evented, unix::EventedFd, Poll, PollOpt, Ready, Token};

use crate::nixtools::stdio::{IStream, OStream};

const BUF_SIZE: usize = 32 * 1024;

#[derive(Debug)]
pub enum Error {
    Sink(io::Error),
    Source(io::Error),
}

type Result = result::Result<usize, Error>;

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

    pub fn gather(&mut self, token: Token) -> Result {
        match self.sources.get(&token) {
            Some(source) => {
                let mut buf = [0; BUF_SIZE];
                let nread = source
                    .borrow_mut()
                    .read(&mut buf)
                    .map_err(|err| Error::Source(err))?;

                self.sink
                    .write_all(&buf[..nread])
                    .map_err(|err| Error::Sink(err))?;
                Ok(nread)
            }

            None => {
                warn!(
                    "[shim] dubious, cannot find source stream for token {:?}",
                    token
                );
                Ok(0)
            }
        }
    }

    pub fn add_source(&mut self, token: Token, source: Rc<RefCell<dyn Read>>) {
        self.sources.insert(token, source);
    }

    pub fn remove_source(&mut self, token: Token) {
        self.sources.remove(&token);
    }
}

#[repr(u8)]
#[derive(Copy, Clone)]
enum ScattererKind {
    STDOUT = 1,
    STDERR = 2,
}

pub struct Scatterer {
    kind: ScattererKind,
    source: IStream,
    sinks: HashMap<usize, Rc<RefCell<dyn Write>>>,
    next_sink_seq_no: usize,
}

impl Scatterer {
    pub fn stdout(source: IStream) -> Self {
        Self::new(ScattererKind::STDOUT, source)
    }

    pub fn stderr(source: IStream) -> Self {
        Self::new(ScattererKind::STDERR, source)
    }

    fn new(kind: ScattererKind, source: IStream) -> Self {
        Self {
            kind: kind,
            source: source,
            sinks: HashMap::new(),
            next_sink_seq_no: 0,
        }
    }

    pub fn scatter(&mut self) -> Result {
        let mut buf = [0; BUF_SIZE];
        let nread = self
            .source
            .read(&mut buf[1..])
            .map_err(|err| Error::Source(err))?;

        buf[0] = self.kind as u8;

        if nread > 0 {
            self.sinks.retain(|idx, writer| {
                match writer.borrow_mut().write_all(&buf[..nread + 1]) {
                    Ok(_) => true,
                    Err(err) => {
                        warn!("[shim] failed to scatter STDIO to sink #{}: {}", idx, err);
                        false
                    }
                }
            });
        }
        Ok(nread)
    }

    pub fn add_sink(&mut self, sink: Rc<RefCell<dyn Write>>) {
        self.sinks.insert(self.next_sink_seq_no, sink);
        self.next_sink_seq_no += 1;
    }
}

impl Evented for Scatterer {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.source.as_raw_fd()).register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.source.as_raw_fd()).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        EventedFd(&self.source.as_raw_fd()).deregister(poll)
    }
}
