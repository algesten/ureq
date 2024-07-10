use core::fmt;
use std::io::{self, Read};

use crate::pool::Connection;
use crate::time::Instant;
use crate::transport::Buffers;
use crate::unit::{Event, Input, Unit};
use crate::Error;

pub struct RecvBody {
    unit: Unit<()>,
    connection: Option<Connection>,
    current_time: Box<dyn Fn() -> Instant + Send + Sync>,
}

impl RecvBody {
    pub(crate) fn new(
        unit: Unit<()>,
        connection: Connection,
        current_time: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        RecvBody {
            unit,
            connection: Some(connection),
            current_time: Box::new(current_time),
        }
    }

    fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let connection = match &mut self.connection {
            Some(v) => v,
            None => return Ok(0),
        };

        let buffers = connection.borrow_buffers(false);
        let event = self.unit.poll_event((self.current_time)(), buffers)?;

        let timeout = match event {
            Event::AwaitInput { timeout } => timeout,
            Event::Reset { must_close } => {
                if let Some(connection) = self.connection.take() {
                    if must_close {
                        connection.close()
                    } else {
                        connection.reuse()
                    }
                }
                return Ok(0);
            }
            _ => unreachable!("expected event AwaitInput"),
        };

        let Buffers { input, .. } = connection.await_input(timeout)?;

        let max = input.len().min(buf.len());
        let input = &input[..max];

        let input_used =
            self.unit
                .handle_input((self.current_time)(), Input::Input { input }, buf)?;

        connection.consume_input(input_used);

        let buffers = connection.borrow_buffers(false);
        let event = self.unit.poll_event((self.current_time)(), buffers)?;

        let output_used = match event {
            Event::ResponseBody { amount } => amount,
            _ => unreachable!("expected event ResponseBody"),
        };

        Ok(output_used)
    }
}

impl Read for RecvBody {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.do_read(buf).map_err(|e| e.into_io())?;

        Ok(n)
    }
}

impl fmt::Debug for RecvBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecvBody").finish()
    }
}
