use std::io;
use std::time::Duration;

use crate::time::Instant;

use super::Transport;

pub struct TransportAdapter {
    pub timeout: Duration,
    pub transport: Box<dyn Transport>,
}
impl TransportAdapter {
    pub(crate) fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            timeout: Instant::duration_until_not_happening(),
            transport,
        }
    }
}

impl io::Read for TransportAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let buffers = self
            .transport
            .await_input(self.timeout)
            .map_err(|e| e.into_io())?;

        let max = buf.len().min(buffers.input.len());
        buf[..max].copy_from_slice(&buffers.input[..max]);
        self.transport.consume_input(max);

        Ok(max)
    }
}

impl io::Write for TransportAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let buffers = self.transport.borrow_buffers(false);

        let max = buf.len().min(buffers.output.len());
        buffers.output[..max].copy_from_slice(&buf[..max]);
        self.transport
            .transmit_output(max, self.timeout)
            .map_err(|e| e.into_io())?;

        Ok(max)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
