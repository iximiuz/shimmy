use std::io;
use std::os::unix::io::AsRawFd;

use mio::{unix::EventedFd, Evented, Poll, PollOpt, Ready, Token};
use nix::sys::{
    signal::{sigprocmask, SigSet, SigmaskHow, Signal},
    signalfd,
};

pub fn signals_block(signals: &[Signal]) -> SigSet {
    let mut oldmask = SigSet::empty();
    sigprocmask(
        SigmaskHow::SIG_BLOCK,
        Some(&sigmask(signals)),
        Some(&mut oldmask),
    )
    .expect("sigprocmask(SIG_BLOCK) failed");
    return oldmask;
}

pub fn signals_restore(mask: &SigSet) {
    sigprocmask(SigmaskHow::SIG_SETMASK, Some(&mask), None)
        .expect("sigprocmask(SIG_SETMASK) failed");
}

pub struct Signalfd(signalfd::SignalFd);

impl Signalfd {
    pub fn new(signals: &[Signal]) -> Self {
        Self(
            signalfd::SignalFd::new(&sigmask(signals))
                .expect(&format!("signalfd() failed for mask {:?}", signals)),
        )
    }

    pub fn read_signal(&mut self) -> Signal {
        match self.0.read_signal() {
            Ok(Some(sinfo)) => {
                Signal::from_c_int(sinfo.ssi_signo as libc::c_int).expect("unexpected signo")
            }
            Ok(None) => panic!("wtf? We are in blocking mode"),
            Err(err) => panic!("read(signalfd) failed {}", err),
        }
    }
}

impl Evented for Signalfd {
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

fn sigmask(signals: &[Signal]) -> SigSet {
    *signals.iter().fold(&mut SigSet::empty(), |mask, sig| {
        mask.add(*sig);
        mask
    })
}
