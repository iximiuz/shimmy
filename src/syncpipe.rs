use std::fs::File;
use std::io::Write;
use std::os::unix::io::{FromRawFd, RawFd};

use serde::Serialize;

use crate::nixtools::kill::TerminationStatus;

#[derive(Serialize)]
struct MessageRuntimeAbnormalTermination {
    kind: &'static str,
    status: String,
    stderr: Vec<u8>,
}

impl MessageRuntimeAbnormalTermination {
    fn new(status: TerminationStatus, stderr: &[u8]) -> Self {
        MessageRuntimeAbnormalTermination {
            kind: "runtime_abnormal_termination",
            status: format!("{}", status),
            stderr: stderr.to_vec(),
        }
    }
}

pub struct SyncPipe(File);

impl SyncPipe {
    pub fn new(fd: RawFd) -> Self {
        SyncPipe(unsafe { File::from_raw_fd(fd) })
    }

    pub fn write_abnormal_runtime_termination(&mut self, status: TerminationStatus, stderr: &[u8]) {
        let message = MessageRuntimeAbnormalTermination::new(status, stderr);
        let encoded = serde_json::to_vec(&message).expect("JSON serialization failed");
        self.0.write_all(&encoded).expect("SyncPipe.write() failed");
    }
}
