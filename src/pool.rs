use agent::Unit;
use std::collections::HashMap;
use std::io::{Read, Result as IoResult};
use stream::Stream;
use url::Url;

#[derive(Default, Debug)]
pub struct ConnectionPool {
    recycle: HashMap<Url, Stream>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        ConnectionPool {
            ..Default::default()
        }
    }

    pub fn try_get_connection(&mut self, url: &Url) -> Option<Stream> {
        self.recycle.remove(url)
    }
}

pub struct PoolReturnRead<R: Read + Sized> {
    unit: Option<Unit>,
    reader: Option<R>,
}

impl<R: Read + Sized> PoolReturnRead<R> {
    pub fn new(unit: Option<Unit>, reader: R) -> Self {
        PoolReturnRead {
            unit,
            reader: Some(reader),
        }
    }

    fn return_connection(&mut self) {
        if let Some(_unit) = self.unit.take() {}
    }

    fn do_read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match self.reader.as_mut() {
            None => return Ok(0),
            Some(reader) => reader.read(buf),
        }
    }
}

impl<R: Read + Sized> Read for PoolReturnRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let amount = self.do_read(buf)?;
        // only if the underlying reader is exhausted can we send a new
        // request to the same socket. hence, we only return it now.
        if amount == 0 {
            self.return_connection();
        }
        Ok(amount)
    }
}
