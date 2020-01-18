use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::rc::Rc;

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

    pub fn write(&mut self, stream: &'static str, buf: &[u8]) -> io::Result<usize> {
        let mut written = 0;
        for line in buf.split(|c| *c == b'\n').filter(|l| l.len() > 0) {
            let message = format!(
                "{} {} {}\n",
                Utc::now().to_rfc3339(),
                stream,
                String::from_utf8_lossy(line)
            );
            written += self.file.write(message.as_bytes())?;
        }
        Ok(written)
    }
}

pub struct Writer {
    logger: Rc<RefCell<Logger>>,
    stream: &'static str,
}

impl Writer {
    pub fn stdout(logger: Rc<RefCell<Logger>>) -> Self {
        Self {
            logger: logger,
            stream: "stdout",
        }
    }

    pub fn stderr(logger: Rc<RefCell<Logger>>) -> Self {
        Self {
            logger: logger,
            stream: "stderr",
        }
    }
}

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.logger.borrow_mut().write(self.stream, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        // noop
        Ok(())
    }
}
