use std::io;

use crate::Error;

pub(crate) struct LimitReader<R> {
    reader: R,
    limit: u64,
    left: u64,
}

impl<R> LimitReader<R> {
    pub fn new(reader: R, limit: u64) -> Self {
        LimitReader {
            reader,
            limit,
            left: limit,
        }
    }
}

impl<R: io::Read> io::Read for LimitReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(Error::BodyExceedsLimit(self.limit).into_io());
        }

        // The max buffer size is usize, which may be 32 bit.
        let max = (self.left.min(usize::MAX as u64) as usize).min(buf.len());

        let n = self.reader.read(&mut buf[..max])?;

        self.left -= n as u64;

        Ok(n)
    }
}
