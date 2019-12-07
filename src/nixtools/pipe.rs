use std::os::unix::io::RawFd;

use log::error;
use nix::fcntl::OFlag;
use nix::unistd::{close, pipe2};

pub struct Pipe {
    rd: RawFd,
    wr: RawFd,
}

impl Pipe {
    pub fn new() -> Self {
        let (rd, wr) = pipe2(OFlag::O_CLOEXEC).expect("pipe2() failed");
        Pipe { rd, wr }
    }

    pub fn rd(&self) -> RawFd {
        self.rd
    }
    pub fn wr(&self) -> RawFd {
        self.wr
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        if let Err(err) = close(self.rd) {
            error!("close({}) pipe.rd failed: {}", self.rd, err);
        }
        if let Err(err) = close(self.wr) {
            error!("close({}) pipe.wr failed: {}", self.wr, err);
        }
    }
}
