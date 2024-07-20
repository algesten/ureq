use std::io;

use crate::pool::Connection;
use crate::transport::time::Instant;
use crate::unit::{Event, Input, Unit};
use crate::Error;

pub(crate) struct UnitHandler {
    unit: Unit<()>,
    connection: Option<Connection>,
    current_time: Box<dyn Fn() -> Instant + Send + Sync>,
}

pub(crate) enum UnitHandlerRef<'a> {
    Shared(&'a mut UnitHandler),
    Owned(UnitHandler),
}

impl<'a> UnitHandlerRef<'a> {
    pub fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        match self {
            UnitHandlerRef::Shared(v) => v.do_read(buf),
            UnitHandlerRef::Owned(v) => v.do_read(buf),
        }
    }
}

impl UnitHandler {
    pub fn new(
        unit: Unit<()>,
        connection: Connection,
        current_time: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        Self {
            unit,
            connection: Some(connection),
            current_time: Box::new(current_time),
        }
    }

    fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut attempts = 0;

        let amount = loop {
            attempts += 1;

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

            let size_before = connection.buffers().input().len();
            connection.await_input(timeout)?;
            let input = connection.buffers().input();
            let size_after = input.len();

            // Did calling await_input result in anymore input in the buffer?
            let made_progress = size_after > size_before;

            let max = size_after.min(buf.len());
            let input = &input[..max];

            let input_used =
                self.unit
                    .handle_input((self.current_time)(), Input::Data { input }, buf)?;
            connection.consume_input(input_used);

            let event = self.unit.poll_event((self.current_time)())?;

            let Event::ResponseBody { amount } = event else {
                unreachable!("Expected event ResponseBody");
            };

            if amount == 0 {
                // unit.handle_input did not manage to process more body data. This
                // might mean we need to fill the input more via connection.await_input().
                // However if we didn't manage to read any bytes last time, the entire
                // chain is stalled.
                if attempts < 3 || made_progress {
                    continue;
                } else {
                    return Err(Error::BodyStalled);
                }
            }

            break amount;
        };

        Ok(amount)
    }
}

impl<'a> io::Read for UnitHandlerRef<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.do_read(buf).map_err(|e| e.into_io())
    }
}
