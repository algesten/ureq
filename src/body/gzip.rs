use std::io;

use flate2::read::MultiGzDecoder;

use crate::error::is_wrapped_ureq_error;
use crate::Error;

pub(crate) struct GzipDecoder<R>(MultiGzDecoder<R>);

impl<R: io::Read> GzipDecoder<R> {
    pub fn new(reader: R) -> Self {
        GzipDecoder(MultiGzDecoder::new(reader))
    }
}

impl<R: io::Read> io::Read for GzipDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf).map_err(|e| {
            if is_wrapped_ureq_error(&e) {
                // If this already is a ureq::Error, like Timeout, pass it along.
                e
            } else {
                Error::Decompress("gzip", e).into_io()
            }
        })
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

    /// Gzip-compress a byte slice using flate2.
    fn gzip_compress(data: &[u8]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    // When ureq transparently decompresses a gzip response, Content-Encoding and
    // Content-Length headers must be stripped from the response per RFC 9110 §8.7.
    // Content-Length no longer matches the decompressed body size, and
    // Content-Encoding no longer applies since the caller receives plaintext.
    #[test]
    fn gz_strips_content_encoding_and_content_length() {
        init_test_log();

        let original = b"{\"hello\":\"world\"}";
        let compressed = gzip_compress(original);
        let compressed_len = compressed.len().to_string();

        set_handler(
            "/gz_strip",
            200,
            &[
                ("content-encoding", "gzip"),
                ("content-length", &compressed_len),
                ("content-type", "application/json"),
            ],
            &compressed,
        );

        let mut res = crate::get("https://my.test/gz_strip").call().unwrap();

        // Content-Encoding must be removed after transparent decompression
        assert!(
            res.headers().get("content-encoding").is_none(),
            "Content-Encoding should be stripped after gzip decompression, got: {:?}",
            res.headers().get("content-encoding"),
        );

        // Content-Length header must be removed (it referred to compressed size)
        assert!(
            res.headers().get("content-length").is_none(),
            "Content-Length header should be stripped after gzip decompression, got: {:?}",
            res.headers().get("content-length"),
        );

        // Body::content_length() must also return None after decompression
        assert!(
            res.body().content_length().is_none(),
            "Body::content_length() should return None after gzip decompression, got: {:?}",
            res.body().content_length(),
        );

        // Other headers must be preserved
        assert_eq!(
            res.headers().get("content-type").unwrap().to_str().unwrap(),
            "application/json",
        );

        // Body must be the decompressed original
        let body = res.body_mut().read_to_string().unwrap();
        assert_eq!(body, "{\"hello\":\"world\"}");
    }
}
