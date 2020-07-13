use crate::body::copy_chunked;
use crate::error::Error;
use crate::response::Response;
use crate::stream::Stream;
use crate::unit::{self, Unit};
use std::io::{Result as IoResult, Write};

pub struct RequestWrite {
    unit: Unit,
    stream: Stream,
    finished: bool,
}

impl RequestWrite {
    pub(crate) fn new(unit: Unit) -> Result<Self, Error> {
        let (stream, _is_recycled) = unit::connect_and_send_prelude(&unit, true, false)?;
        Ok(RequestWrite {
            unit,
            stream,
            finished: false,
        })
    }

    // This should only ever be called once either explicitly in finish() or when dropped
    fn do_finish(&mut self) -> Response {
        assert!(!self.finished);
        self.finished = true;
        if self.unit.is_chunked {
            // send empty chunk to signal end of chunks
            let mut empty: &[u8] = &[];
            let _ = copy_chunked(&mut empty, &mut self.stream, true);
        }
        let resp = Response::from_read(&mut self.stream);
        // squirrel away cookies
        unit::save_cookies(&self.unit, &resp);

        resp
    }
    pub fn finish(mut self) -> Response {
        self.do_finish()
    }
}

impl Write for RequestWrite {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        if self.unit.is_chunked {
            let mut chunk = buf;
            copy_chunked(&mut chunk, &mut self.stream, false).map(|s| s as usize)
        } else {
            self.stream.write(buf)
        }
    }

    fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
        self.stream.flush()
    }
}

impl Drop for RequestWrite {
    fn drop(&mut self) {
        if !self.finished {
            self.do_finish();
        }
    }
}

impl ::std::fmt::Debug for RequestWrite {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(f, "RequestWrite({} {})", self.unit.method, self.unit.url)
    }
}
