use crate::nixtools::{create_pipe, IOStream, IOStreams};

pub struct StdioPipes {
    pub slave: IOStreams,
    pub master: IOStreams,
}

impl StdioPipes {
    pub fn new() -> StdioPipes {
        let stdout = create_pipe();
        let stderr = create_pipe();
        StdioPipes {
            slave: IOStreams {
                In: IOStream::DevNull,
                Out: IOStream::Fd(stdout.wr()),
                Err: IOStream::Fd(stderr.wr()),
            },
            master: IOStreams {
                In: IOStream::DevNull,
                Out: IOStream::Fd(stdout.rd()),
                Err: IOStream::Fd(stderr.rd()),
            },
        }
    }
}
