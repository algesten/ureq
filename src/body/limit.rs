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

#[cfg(all(test, feature = "_test"))]
mod test {
    use std::io;

    use crate::test::init_test_log;
    use crate::transport::set_handler;
    use crate::Error;

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
}
