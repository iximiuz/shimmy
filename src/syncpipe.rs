use std::fs::File;
use std::io::Write;
use std::os::unix::io::{FromRawFd, RawFd};

use crate::nixtools::kill::TerminationStatus;

pub struct SyncPipe(File);

impl SyncPipe {
    pub fn new(fd: RawFd) -> Self {
        SyncPipe(unsafe { File::from_raw_fd(fd) })
    }

    pub fn write_abnormal_runtime_termination(&mut self, status: TerminationStatus, stderr: &[u8]) {
        // TODO: serialize status
        self.0.write_all(stderr);
    }
}
