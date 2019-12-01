use nix::sys::signal::{sigprocmask, SigSet, SigmaskHow, Signal};
use nix::sys::signalfd;

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

pub struct Signalfd {
    sfd: signalfd::SignalFd,
}

impl Signalfd {
    pub fn new(signals: &[Signal]) -> Self {
        Self {
            sfd: signalfd::SignalFd::new(&sigmask(signals))
                .expect(&format!("signalfd() failed for mask {:?}", signals)),
        }
    }

    pub fn read_signal(&mut self) -> Signal {
        match self.sfd.read_signal() {
            Ok(Some(sinfo)) => {
                Signal::from_c_int(sinfo.ssi_signo as libc::c_int).expect("unexpected signo")
            }
            Ok(None) => panic!("wtf? We are in blocking mode"),
            Err(err) => panic!("read(signalfd) failed {}", err),
        }
    }
}

fn sigmask(signals: &[Signal]) -> SigSet {
    *signals.iter().fold(&mut SigSet::empty(), |mask, sig| {
        mask.add(*sig);
        mask
    })
}
