use std::io::{self, Read};

use crate::pool::Connection;
use crate::time::Instant;
use crate::transport::Buffers;
use crate::unit::{Event, Input, Unit};
use crate::Error;

pub struct RecvBody {
    unit: Unit<()>,
    connection: Connection,
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
            connection,
            current_time: Box::new(current_time),
        }
    }

    fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let buffers = self.connection.borrow_buffers();
        let event = self.unit.poll_event((self.current_time)(), buffers)?;

        let timeout = match event {
            Event::AwaitInput { timeout, is_body } => {
                assert!(is_body);
                timeout
            }
            _ => unreachable!("expected event AwaitInput"),
        };

        let Buffers { input, .. } = self.connection.await_input(timeout, true)?;
        let input_used =
            self.unit
                .handle_input((self.current_time)(), Input::Input { input }, buf)?;
        self.connection.consume_input(input_used);

        let buffers = self.connection.borrow_buffers();
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
        self.do_read(buf).map_err(|e| e.into_io())?;

        Ok(0)
    }
}
