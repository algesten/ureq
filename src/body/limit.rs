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
        if buf.is_empty() {
            return Ok(0);
        }

        if self.left == 0 {
            // A body exactly `limit` bytes long is allowed: only fail if the
            // inner reader has more data, not when it is at the end.
            let mut probe = [0_u8; 1];
            if self.reader.read(&mut probe)? == 0 {
                return Ok(0);
            }
            return Err(Error::BodyExceedsLimit(self.limit).into_io());
        }

        // The max buffer size is usize, which may be 32 bit.
        let max = (self.left.min(usize::MAX as u64) as usize).min(buf.len());

        let n = self.reader.read(&mut buf[..max])?;

        self.left -= n as u64;

        Ok(n)
    }
}

#[cfg(all(test, feature = "_test"))]
mod test {
    use std::io;

    use crate::Error;
    use crate::test::init_test_log;
    use crate::transport::set_handler;

    #[test]
    fn short_read() {
        init_test_log();
        set_handler("/get", 200, &[("content-length", "10")], b"hello");
        let mut res = crate::get("https://my.test/get").call().unwrap();
        let err = res.body_mut().read_to_string().unwrap_err();
        let ioe = err.into_io();
        assert_eq!(ioe.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn limit_below_size() {
        init_test_log();
        set_handler("/get", 200, &[("content-length", "5")], b"hello");
        let mut res = crate::get("https://my.test/get").call().unwrap();
        let err = res
            .body_mut()
            .with_config()
            .limit(3)
            .read_to_string()
            .unwrap_err();
        println!("{:?}", err);
        assert!(matches!(err, Error::BodyExceedsLimit(3)));
    }

    #[test]
    #[cfg(feature = "gzip")]
    fn limit_applies_to_decompressed_size() {
        use std::io::Write;
        init_test_log();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        enc.write_all(&[0_u8; 10_000]).unwrap();
        let gz = enc.finish().unwrap();
        assert!(gz.len() < 100);
        let len = gz.len().to_string();
        set_handler(
            "/get",
            200,
            &[("content-encoding", "gzip"), ("content-length", &len)],
            &gz,
        );
        let mut res = crate::get("https://my.test/get").call().unwrap();
        let err = res
            .body_mut()
            .with_config()
            .limit(1_000)
            .read_to_vec()
            .unwrap_err();
        assert!(matches!(err, Error::BodyExceedsLimit(1_000)));
    }

    #[test]
    fn empty_read_at_limit_does_not_consume() {
        use std::io::Read;
        init_test_log();
        set_handler("/get", 200, &[("content-length", "6")], b"hello!");
        let mut res = crate::get("https://my.test/get").call().unwrap();
        let mut reader = res.body_mut().with_config().limit(5).reader();
        let mut buf = [0_u8; 5];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        let err = reader.read(&mut [0_u8; 8]).unwrap_err();
        assert!(err.to_string().contains("larger than request limit"));
    }

    #[test]
    fn limit_equal_to_size() {
        init_test_log();
        set_handler("/get", 200, &[("content-length", "5")], b"hello");
        let mut res = crate::get("https://my.test/get").call().unwrap();
        let s = res
            .body_mut()
            .with_config()
            .limit(5)
            .read_to_string()
            .unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn limit_equal_to_size_chunked() {
        init_test_log();
        set_handler(
            "/get",
            200,
            &[("transfer-encoding", "chunked")],
            b"5\r\nhello\r\n0\r\n\r\n",
        );
        let mut res = crate::get("https://my.test/get").call().unwrap();
        let s = res
            .body_mut()
            .with_config()
            .limit(5)
            .read_to_string()
            .unwrap();
        assert_eq!(s, "hello");
    }
}
