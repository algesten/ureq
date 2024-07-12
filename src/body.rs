use core::fmt;
use std::io::{self, Read};

use crate::pool::Connection;
use crate::time::Instant;
use crate::unit::{Event, Input, Unit};
use crate::Error;

pub struct Body {
    unit: Unit<()>,
    connection: Option<Connection>,
    current_time: Box<dyn Fn() -> Instant + Send + Sync>,
}

impl Body {
    pub(crate) fn new(
        unit: Unit<()>,
        connection: Connection,
        current_time: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        Body {
            unit,
            connection: Some(connection),
            current_time: Box::new(current_time),
        }
    }

    pub fn as_reader(&mut self, limit: u64) -> BodyReader {
        BodyReader::shared(self, limit)
    }

    pub fn into_reader(self, limit: u64) -> BodyReader<'static> {
        BodyReader::owned(self, limit)
    }

    pub fn read_to_string(&mut self, limit: usize) -> Result<String, Error> {
        let mut buf = String::new();
        let mut reader = self.as_reader(limit as u64);
        reader.read_to_string(&mut buf)?;
        Ok(buf)
    }

    pub fn read_to_vec(&mut self, limit: usize) -> Result<Vec<u8>, Error> {
        let mut buf = Vec::new();
        let mut reader = self.as_reader(limit as u64);
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let now = (self.current_time)();

        let Some(connection) = &mut self.connection else {
            return Ok(0);
        };

        let event = self.unit.poll_event((self.current_time)())?;

        let timeout = match event {
            Event::AwaitInput { timeout } => timeout,
            Event::Reset { must_close } => {
                if let Some(connection) = self.connection.take() {
                    if must_close {
                        connection.close()
                    } else {
                        connection.reuse(now)
                    }
                }
                return Ok(0);
            }
            _ => unreachable!("Expected event AwaitInput"),
        };

        connection.await_input(timeout)?;
        let input = connection.buffers().input();

        let max = input.len().min(buf.len());
        let input = &input[..max];

        let input_used =
            self.unit
                .handle_input((self.current_time)(), Input::Data { input }, buf)?;

        connection.consume_input(input_used);

        let event = self.unit.poll_event((self.current_time)())?;

        let Event::ResponseBody { amount } = event else {
            unreachable!("Expected event ResponseBody");
        };

        Ok(amount)
    }
}

pub struct BodyReader<'a> {
    body: BodyRef<'a>,
    left: u64,
}

enum BodyRef<'a> {
    Shared(&'a mut Body),
    Owned(Body),
}

impl<'a> BodyReader<'a> {
    fn shared(body: &'a mut Body, limit: u64) -> BodyReader<'a> {
        Self {
            body: BodyRef::Shared(body),
            left: limit,
        }
    }
}

impl BodyReader<'static> {
    fn owned(body: Body, limit: u64) -> BodyReader<'static> {
        Self {
            body: BodyRef::Owned(body),
            left: limit,
        }
    }
}

impl<'a> BodyRef<'a> {
    fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        match self {
            BodyRef::Shared(v) => v.do_read(buf),
            BodyRef::Owned(v) => v.do_read(buf),
        }
    }
}

impl<'a> Read for BodyReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(Error::BodyExceedsLimit.into_io());
        }

        // The max buffer size is usize, which may be 32 bit.
        let max = (self.left.min(usize::MAX as u64) as usize).min(buf.len());

        let n = self
            .body
            .do_read(&mut buf[..max])
            .map_err(|e| e.into_io())?;

        self.left -= n as u64;

        Ok(n)
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Body").finish()
    }
}
