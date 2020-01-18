use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use chrono::Utc;

pub struct Logger {
    file: File,
}

impl Logger {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            file: OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .unwrap(),
        }
    }

    pub fn write_container_stdout(&mut self, data: &[u8]) {
        self.write("stdout", data);
    }

    pub fn write_container_stderr(&mut self, data: &[u8]) {
        self.write("stderr", data);
    }

    fn write(&mut self, stream: &'static str, data: &[u8]) {
        for line in data.split(|c| *c == b'\n').filter(|l| l.len() > 0) {
            write!(
                self.file,
                "{} {} {}\n",
                Utc::now().to_rfc3339(),
                stream,
                String::from_utf8_lossy(line)
            )
            .expect("container log write failed");
        }
    }
}
