use std::io;

use crate::Error;

use super::handler::{UnitHandler, UnitHandlerRef};

pub(crate) struct LimitReader<'a> {
    unit_handler: UnitHandlerRef<'a>,
    left: u64,
}

impl<'a> LimitReader<'a> {
    pub fn shared(u: &'a mut UnitHandler, limit: u64) -> LimitReader<'a> {
        Self {
            unit_handler: UnitHandlerRef::Shared(u),
            left: limit,
        }
    }
}

impl LimitReader<'static> {
    pub fn owned(u: UnitHandler, limit: u64) -> LimitReader<'static> {
        Self {
            unit_handler: UnitHandlerRef::Owned(u),
            left: limit,
        }
    }
}

impl<'a> io::Read for LimitReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(Error::BodyExceedsLimit.into_io());
        }

        // The max buffer size is usize, which may be 32 bit.
        let max = (self.left.min(usize::MAX as u64) as usize).min(buf.len());

        let n = self
            .unit_handler
            .do_read(&mut buf[..max])
            .map_err(|e| e.into_io())?;

        self.left -= n as u64;

        Ok(n)
    }
}
