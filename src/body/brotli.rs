use std::io::{self, Read};

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
        self.read_to_body_end(buf).map_err(|e| {
            if is_wrapped_ureq_error(&e) {
                // If this already is a ureq::Error, like Timeout, pass it along.
                e
            } else {
                Error::Decompress("brotli", e).into_io()
            }
        })
    }
}

impl<R: io::Read> BrotliDecoder<R> {
    // A brotli stream marks its own end, and the decompressor stops reading its input
    // there. The body must still be read to its end: that is what detects a body cut
    // short of its content-length or last chunk, and what returns the connection to
    // the pool.
    fn read_to_body_end(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.0.read(buf)?;
        if n > 0 || buf.is_empty() {
            return Ok(n);
        }

        // Reading again at the end of the stream fails if the decompressor has
        // buffered input left over after the stream.
        let n = self.0.read(buf)?;
        if n > 0 {
            return Ok(n);
        }

        // Nothing buffered, so the body itself must be at the end too.
        let mut probe = [0_u8; 1];
        if self.0.get_mut().read(&mut probe)? > 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "data after end of brotli stream",
            ));
        }

        Ok(0)
    }
}

#[cfg(all(test, feature = "_test"))]
mod test {
    use crate::test::init_test_log;
    use crate::transport::set_handler;
    use crate::{Agent, Error};

    // "hello world" as one uncompressed meta-block followed by an empty last meta-block.
    const BR_HELLO_WORLD: &[u8] = b"\xA0\x00\x10hello world\x03";

    // Test that a stream gets returned to the pool if it is brotli encoded. The brotli
    // decoder knows where the stream ends and does not by itself read on to the 0 chunk.
    #[test]
    fn br_internal_length() {
        init_test_log();

        let mut br_body = b"F\r\n".to_vec();
        br_body.extend_from_slice(BR_HELLO_WORLD);
        br_body.extend_from_slice(b"\r\n0\r\n\r\n");

        let agent = Agent::new_with_defaults();
        assert_eq!(agent.pool_count(), 0);

        set_handler(
            "/br_body",
            200,
            &[("transfer-encoding", "chunked"), ("content-encoding", "br")],
            &br_body,
        );

        let mut res = agent.get("https://example.test/br_body").call().unwrap();
        assert_eq!(res.body_mut().read_to_string().unwrap(), "hello world");

        assert_eq!(agent.pool_count(), 1);
    }

    // A complete brotli stream does not make up for a body that is shorter than
    // the declared content-length.
    #[test]
    fn br_short_content_length() {
        init_test_log();

        set_handler(
            "/br_short",
            200,
            &[("content-encoding", "br"), ("content-length", "100")],
            BR_HELLO_WORLD,
        );

        let mut res = crate::get("https://my.test/br_short").call().unwrap();
        let err = res.body_mut().read_to_string().unwrap_err();
        assert!(matches!(err, Error::Decompress("brotli", _)), "{:?}", err);
    }

    // Same, for a chunked body without the last chunk.
    #[test]
    fn br_short_chunked() {
        init_test_log();

        let mut br_body = b"F\r\n".to_vec();
        br_body.extend_from_slice(BR_HELLO_WORLD);
        br_body.extend_from_slice(b"\r\n");

        set_handler(
            "/br_short_chunked",
            200,
            &[("transfer-encoding", "chunked"), ("content-encoding", "br")],
            &br_body,
        );

        let mut res = crate::get("https://my.test/br_short_chunked")
            .call()
            .unwrap();
        let err = res.body_mut().read_to_string().unwrap_err();
        assert!(matches!(err, Error::Decompress("brotli", _)), "{:?}", err);
    }

    // Data following the brotli stream is an error, whether or not the decompressor
    // has already buffered it.
    #[test]
    fn br_trailing_data() {
        init_test_log();

        for (path, trailing) in [("/br_trailing_10", 10), ("/br_trailing_10k", 10_000)] {
            let mut br_body = BR_HELLO_WORLD.to_vec();
            br_body.resize(br_body.len() + trailing, b'x');
            let len = br_body.len().to_string();

            set_handler(
                path,
                200,
                &[("content-encoding", "br"), ("content-length", &len)],
                &br_body,
            );

            let mut res = crate::get(format!("https://my.test{}", path))
                .call()
                .unwrap();
            let err = res.body_mut().read_to_string().unwrap_err();
            assert!(matches!(err, Error::Decompress("brotli", _)), "{:?}", err);
        }
    }
}
