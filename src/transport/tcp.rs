use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use std::{fmt, io};

use crate::time::DurationExt;
use crate::util::IoResultExt;
use crate::Error;

use super::{Buffers, ConnectionDetails, Connector, LazyBuffers, Transport};

pub struct TcpConnector {}

impl Connector for TcpConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, crate::Error> {
        if chained.is_some() {
            // The chained connection overrides whatever we were to open here.
            // In the DefaultConnector chain this would be a SOCKS proxy connection.
            return Ok(chained);
        }

        let stream = TcpStream::connect_timeout(&details.addr, details.timeout)?;
        let config = &details.config;
        let buffers = LazyBuffers::new(config.input_buffer_size, config.output_buffer_size);
        let transport = TcpTransport::new(stream, buffers);

        Ok(Some(Box::new(transport)))
    }
}

pub struct TcpTransport {
    stream: TcpStream,
    buffers: LazyBuffers,
    timeout_write: Option<Duration>,
    timeout_read: Option<Duration>,
}

impl TcpTransport {
    pub fn new(stream: TcpStream, buffers: LazyBuffers) -> TcpTransport {
        TcpTransport {
            stream,
            buffers,
            timeout_read: None,
            timeout_write: None,
        }
    }
}

// The goal here is to only cause a syscall to set the timeout if it's necessary.
fn maybe_update_timeout(
    timeout: Duration,
    previous: &mut Option<Duration>,
    stream: &TcpStream,
    f: impl Fn(&TcpStream, Option<Duration>) -> io::Result<()>,
) -> io::Result<()> {
    let maybe_timeout = if timeout.is_zero() || timeout.is_not_happening() {
        None
    } else {
        Some(timeout)
    };

    if maybe_timeout != *previous {
        (f)(stream, maybe_timeout)?;
        *previous = maybe_timeout;
    }

    Ok(())
}

impl Transport for TcpTransport {
    fn borrow_buffers(&mut self) -> Buffers {
        // Assume the borrower wants to use the input. Assert we don't have unconsumed content.
        self.buffers.assert_and_clear_input_filled();

        self.buffers.borrow_mut()
    }

    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error> {
        maybe_update_timeout(
            timeout,
            &mut self.timeout_write,
            &self.stream,
            TcpStream::set_write_timeout,
        )?;

        let buffers = self.buffers.borrow_mut();
        let output = &buffers.output[..amount];
        self.stream.write_all(output).normalize_would_block()?;

        Ok(())
    }

    fn await_input(&mut self, timeout: Duration, _is_body: bool) -> Result<Buffers, Error> {
        // There might be input left from the previous await_input.
        if self.buffers.unconsumed() > 0 {
            return Ok(self.borrow_buffers());
        }

        // Proceed to fill the buffers from the TcpStream
        maybe_update_timeout(
            timeout,
            &mut self.timeout_read,
            &self.stream,
            TcpStream::set_read_timeout,
        )?;

        // Ensure we get the entire input buffer to write to.
        self.buffers.assert_and_clear_input_filled();

        let buffers = self.buffers.borrow_mut();
        let amount = self.stream.read(buffers.input).normalize_would_block()?;

        // Cap the input buffer.
        self.buffers.set_input_filled(amount);

        Ok(self.borrow_buffers())
    }

    fn consume_input(&mut self, amount: usize) {
        self.buffers.consume_input(amount);
    }
}

impl fmt::Debug for TcpConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpConnector").finish()
    }
}

impl fmt::Debug for TcpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpTransport")
            .field("addr", &self.stream.peer_addr().ok())
            .finish()
    }
}
