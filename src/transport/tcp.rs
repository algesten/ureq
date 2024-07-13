use std::io::{Read, Write};
use std::net::TcpStream;
use std::{fmt, io, time};

use crate::time::Duration;
use crate::util::IoResultExt;
use crate::Error;

use super::{Buffers, ConnectionDetails, Connector, LazyBuffers, Transport};

pub struct TcpConnector;

impl Connector for TcpConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, crate::Error> {
        if chained.is_some() {
            // The chained connection overrides whatever we were to open here.
            // In the DefaultConnector chain this would be a SOCKS proxy connection.
            trace!("Skip");
            return Ok(chained);
        }

        trace!("Try connect TcpStream to {}", details.addr);

        let stream = TcpStream::connect_timeout(&details.addr, *details.timeout)?;

        if details.config.no_delay {
            stream.set_nodelay(true)?;
        }

        let config = &details.config;
        let buffers = LazyBuffers::new(config.input_buffer_size, config.output_buffer_size);
        let transport = TcpTransport::new(stream, buffers);

        debug!("Connected TcpStream to {}", details.addr);

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
    f: impl Fn(&TcpStream, Option<time::Duration>) -> io::Result<()>,
) -> io::Result<()> {
    let maybe_timeout = if timeout.is_zero() || timeout.is_not_happening() {
        None
    } else {
        Some(timeout)
    };

    if maybe_timeout != *previous {
        (f)(stream, maybe_timeout.map(|t| *t))?;
        *previous = maybe_timeout;
    }

    Ok(())
}

impl Transport for TcpTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error> {
        maybe_update_timeout(
            timeout,
            &mut self.timeout_write,
            &self.stream,
            TcpStream::set_write_timeout,
        )?;

        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output).normalize_would_block()?;

        Ok(())
    }

    fn await_input(&mut self, timeout: Duration) -> Result<(), Error> {
        if self.buffers.can_use_input() {
            return Ok(());
        }

        // Proceed to fill the buffers from the TcpStream
        maybe_update_timeout(
            timeout,
            &mut self.timeout_read,
            &self.stream,
            TcpStream::set_read_timeout,
        )?;

        let input = self.buffers.input_mut();
        let amount = self.stream.read(input)?;
        self.buffers.add_filled(amount);

        Ok(())
    }

    fn consume_input(&mut self, amount: usize) {
        self.buffers.consume(amount);
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
