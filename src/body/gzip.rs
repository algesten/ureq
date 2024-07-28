use std::io;

use flate2::read::MultiGzDecoder;

use crate::Error;

pub(crate) struct GzipDecoder<R>(MultiGzDecoder<R>);

impl<R: io::Read> GzipDecoder<R> {
    pub fn new(reader: R) -> Self {
        GzipDecoder(MultiGzDecoder::new(reader))
    }
}

impl<R: io::Read> io::Read for GzipDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0
            .read(buf)
            .map_err(|e| Error::Decompress("gzip", e).into_io())
    }
}

#[cfg(all(test, feature = "_test"))]
mod test {
    use crate::test::init_test_log;
    use crate::transport::set_handler;
    use crate::Agent;

    // Test that a stream gets returned to the pool if it is gzip encoded and the gzip
    // decoder reads the exact amount from a chunked stream, not past the 0. This
    // happens because gzip has built-in knowledge of the length to read.
    #[test]
    fn gz_internal_length() {
        init_test_log();

        let gz_body = vec![
            b'E', b'\r', b'\n', // 14 first chunk
            0x1F, 0x8B, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0xCB, 0x48, 0xCD, 0xC9,
            b'\r', b'\n', //
            b'E', b'\r', b'\n', // 14 second chunk
            0xC9, 0x57, 0x28, 0xCF, 0x2F, 0xCA, 0x49, 0x51, 0xC8, 0x18, 0xBC, 0x6C, 0x00, 0xA5,
            b'\r', b'\n', //
            b'7', b'\r', b'\n', // 7 third chunk
            0x5C, 0x7C, 0xEF, 0xA7, 0x00, 0x00, 0x00, //
            b'\r', b'\n', //
            // end
            b'0', b'\r', b'\n', //
            b'\r', b'\n', //
        ];

        let agent = Agent::new_with_defaults();
        assert_eq!(agent.pool_count(), 0);

        set_handler(
            "/gz_body",
            200,
            &[
                ("transfer-encoding", "chunked"),
                ("content-encoding", "gzip"),
            ],
            &gz_body,
        );

        let mut res = agent.get("https://example.test/gz_body").call().unwrap();
        res.body_mut().read_to_string().unwrap();

        assert_eq!(agent.pool_count(), 1);
    }
}
