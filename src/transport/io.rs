use std::io;

use crate::time::{Duration, NextTimeout};
use crate::TimeoutReason;

use super::Transport;

pub struct TransportAdapter {
    pub timeout: NextTimeout,
    pub transport: Box<dyn Transport>,
}

impl TransportAdapter {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            timeout: NextTimeout {
                after: Duration::NotHappening,
                reason: TimeoutReason::Global,
            },
            transport,
        }
    }

    pub fn get_ref(&self) -> &dyn Transport {
        &*self.transport
    }

    pub fn get_mut(&mut self) -> &mut dyn Transport {
        &mut *self.transport
    }
}

impl io::Read for TransportAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.transport
            .await_input(self.timeout)
            .map_err(|e| e.into_io())?;
        let input = self.transport.buffers().input();

        let max = buf.len().min(input.len());
        buf[..max].copy_from_slice(&input[..max]);
        self.transport.consume_input(max);

        Ok(max)
    }
}

impl io::Write for TransportAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let output = self.transport.buffers().output_mut();

        let max = buf.len().min(output.len());
        output[..max].copy_from_slice(&buf[..max]);
        self.transport
            .transmit_output(max, self.timeout)
            .map_err(|e| e.into_io())?;

        Ok(max)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
