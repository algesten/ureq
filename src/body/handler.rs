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
        let Some(connection) = &mut self.connection else {
            return Ok(0);
        };

        let has_buffered_input = connection.buffers().can_use_input();

        // Each read to the underlying buffers needs to be kept in sync with the
        // unit state. The first poll should be event AwaitInput or Reset.
        let event = self.unit.poll_event((self.current_time)())?;

        let timeout = match event {
            Event::AwaitInput { timeout } => timeout,
            Event::Reset { must_close } => {
                if let Some(connection) = self.connection.take() {
                    if must_close {
                        trace!("Must close");
                        connection.close()
                    } else if has_buffered_input {
                        debug!("Close due to excess body data");
                        connection.close()
                    } else {
                        trace!("Attempt reuse");
                        connection.reuse((self.current_time)())
                    }
                }
                return Ok(0);
            }
            _ => unreachable!("Expected event AwaitInput or Reset"),
        };

        // Can we use content that is already buffered?
        if has_buffered_input {
            let amount = ship_input(connection, &mut self.unit, &self.current_time, buf)?;

            // The body parser might not get enough input to make progress (such as when
            // reading a chunked body and not getting the entire chunk length). In such
            // case we fall through to a regular read.
            if amount > 0 {
                return Ok(amount);
            }
        }

        connection.await_input(timeout)?;

        ship_input(connection, &mut self.unit, &self.current_time, buf)
    }
}

fn ship_input(
    connection: &mut Connection,
    unit: &mut Unit<()>,
    current_time: &(dyn Fn() -> Instant + Send + Sync),
    buf: &mut [u8],
) -> Result<usize, Error> {
    let input = connection.buffers().input();
    let input_used = unit.handle_input((current_time)(), Input::Data { input }, buf)?;
    connection.consume_input(input_used);

    let event = unit.poll_event((current_time)())?;

    let Event::ResponseBody { amount } = event else {
        unreachable!("Expected event ResponseBody");
    };

    Ok(amount)
}

impl<'a> io::Read for UnitHandlerRef<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.do_read(buf).map_err(|e| e.into_io())
    }
}
