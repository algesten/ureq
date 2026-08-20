use std::io;

use brotli_decompressor::Decompressor;

use crate::Error;
use crate::error::is_wrapped_ureq_error;

pub(crate) struct BrotliDecoder<R: io::Read>(Decompressor<R>);

impl<R: io::Read> BrotliDecoder<R> {
    pub fn new(reader: R) -> Self {
        BrotliDecoder(Decompressor::new(reader, 4096))
    }
}

impl<R: io::Read> io::Read for BrotliDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf).map_err(|e| {
            if is_wrapped_ureq_error(&e) {
                // If this already is a ureq::Error, like Timeout, pass it along.
                e
            } else {
                Error::Decompress("brotli", e).into_io()
            }
        })
    }
}
