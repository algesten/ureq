use std::io::{Read, Write};
use std::net::TcpStream;
use std::{fmt, io, time};

use crate::time::{Duration, NextTimeout};
use crate::util::IoResultExt;
use crate::Error;

use super::{Buffers, ConnectionDetails, Connector, LazyBuffers, Transport};

/// Connector for regular TCP sockets.
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

        let timeout = details.timeout;

        let maybe_stream = if let Some(when) = timeout.not_zero() {
            TcpStream::connect_timeout(&details.addr, *when)
        } else {
            TcpStream::connect(details.addr)
        }
        .normalize_would_block();

        let stream = match maybe_stream {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                return Err(Error::Timeout(timeout.reason))
            }
            Err(e) => return Err(e.into()),
        };

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
    timeout: NextTimeout,
    previous: &mut Option<Duration>,
    stream: &TcpStream,
    f: impl Fn(&TcpStream, Option<time::Duration>) -> io::Result<()>,
) -> io::Result<()> {
    let maybe_timeout = timeout.not_zero();

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

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
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

    fn await_input(&mut self, timeout: NextTimeout) -> Result<(), Error> {
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

    fn is_open(&mut self) -> bool {
        probe_tcp_stream(&mut self.stream).unwrap_or(false)
    }
}

fn probe_tcp_stream(stream: &mut TcpStream) -> Result<bool, Error> {
    // Temporary do non-blocking IO
    stream.set_nonblocking(true)?;

    let mut buf = [0];
    match stream.read(&mut buf) {
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
            // This is the correct condition. There should be no waiting
            // bytes, and therefore reading would block
        }
        // Any bytes read means the server sent some garbage we didn't ask for
        Ok(_) => return Ok(false),
        // Errors such as closed connection
        Err(_) => return Ok(false),
    };

    // Reset back to blocking
    stream.set_nonblocking(false)?;

    Ok(true)
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
