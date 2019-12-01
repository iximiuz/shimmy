use std::os::unix::io::RawFd;

use nix::fcntl::OFlag;
use nix::unistd::pipe2;

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
