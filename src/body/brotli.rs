use std::io;

use brotli_decompressor::Decompressor;

use crate::Error;

pub(crate) struct BrotliDecoder<R: io::Read>(Decompressor<R>);

impl<R: io::Read> BrotliDecoder<R> {
    pub fn new(reader: R) -> Self {
        BrotliDecoder(Decompressor::new(reader, 4096))
    }
}

impl<R: io::Read> io::Read for BrotliDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0
            .read(buf)
            .map_err(|e| Error::Decompress("brotli", e).into_io())
    }
}
