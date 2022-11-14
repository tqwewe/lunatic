use std::io::{self, Write};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventsScratch {
    pub read_ptr: usize,
    pub buffer: Vec<u8>,
}

impl EventsScratch {
    pub fn new(buffer: Vec<u8>) -> Self {
        EventsScratch {
            read_ptr: 0,
            buffer,
        }
    }
}

impl io::Read for EventsScratch {
    fn read(&mut self, mut buf: &mut [u8]) -> std::io::Result<usize> {
        let slice = if let Some(slice) = self.buffer.get(self.read_ptr..) {
            slice
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "Reading outside message buffer",
            ));
        };
        let bytes = buf.write(slice)?;
        self.read_ptr += bytes;
        Ok(bytes)
    }
}
