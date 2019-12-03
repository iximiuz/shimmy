use std::fs::File;
use std::io::Write;
use std::os::unix::io::{FromRawFd, RawFd};

use serde::Serialize;

use crate::runtime::TerminationStatus;

#[derive(Serialize)]
struct MessageRuntimeAbnormalTermination {
    kind: &'static str,
    status: String,
    stderr: String,
}

impl MessageRuntimeAbnormalTermination {
    fn new(status: TerminationStatus, stderr: &[u8]) -> Self {
        MessageRuntimeAbnormalTermination {
            kind: "runtime_abnormal_termination",
            status: format!("{}", status),
            stderr: String::from_utf8(stderr.to_vec()).unwrap_or(format!("{:?}", stderr)),
        }
    }
}

pub struct SyncPipe(File);

impl SyncPipe {
    pub fn new(fd: RawFd) -> Self {
        SyncPipe(unsafe { File::from_raw_fd(fd) })
    }

    pub fn report_abnormal_runtime_termination(
        &mut self,
        status: TerminationStatus,
        stderr: &[u8],
    ) {
        let msg = serde_json::to_vec(&MessageRuntimeAbnormalTermination::new(status, stderr))
            .expect("JSON serialization failed");
        self.0.write_all(&msg).expect("SyncPipe.write() failed");
    }
}
