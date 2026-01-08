use std::io;

use crate::Timeout;

use super::time::Duration;
use super::{NextTimeout, Transport};

/// Helper to turn a [`Transport`] into a std::io [`Read`](io::Read) and [`Write`](io::Write).
///
/// This is useful when integrating with components that expect a regular `Read`/`Write`. In
/// ureq this is used both for the [`RustlsConnector`](crate::unversioned::transport::RustlsConnector) and the
/// [`NativeTlsConnector`](crate::unversioned::transport::NativeTlsConnector).
pub struct TransportAdapter<T: Transport = Box<dyn Transport>> {
    timeout: NextTimeout,
    transport: T,
}

impl<T: Transport> TransportAdapter<T> {
    /// Creates a new adapter
    pub fn new(transport: T) -> Self {
        Self {
            timeout: NextTimeout {
                after: Duration::NotHappening,
                reason: Timeout::Global,
            },
            transport,
        }
    }

    /// Set a new value of the timeout.
    pub fn set_timeout(&mut self, timeout: NextTimeout) {
        self.timeout = timeout;
    }

    /// Reference to the adapted transport
    pub fn get_ref(&self) -> &dyn Transport {
        &self.transport
    }

    /// Mut reference to the adapted transport
    pub fn get_mut(&mut self) -> &mut dyn Transport {
        &mut self.transport
    }

    /// Reference to the inner transport.
    pub fn inner(&self) -> &dyn Transport {
        &self.transport
    }

    /// Turn the adapter back into the wrapped transport
    pub fn into_inner(self) -> T {
        self.transport
    }
}

impl<T: Transport> io::Read for TransportAdapter<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.transport
            .maybe_await_input(self.timeout)
            .map_err(|e| e.into_io())?;
        let input = self.transport.buffers().input();

        let max = buf.len().min(input.len());
        buf[..max].copy_from_slice(&input[..max]);
        self.transport.buffers().input_consume(max);

        Ok(max)
    }
}

impl<T: Transport> io::Write for TransportAdapter<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let output = self.transport.buffers().output();

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

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use super::super::*;
    use super::*;

    #[test]
    fn timeout_errors_translate_transparently() {
        #[derive(Debug)]
        struct FailingTransport(LazyBuffers);
        impl Transport for FailingTransport {
            fn buffers(&mut self) -> &mut dyn crate::unversioned::transport::Buffers {
                &mut self.0
            }

            fn transmit_output(
                &mut self,
                _amount: usize,
                _timeout: NextTimeout,
            ) -> Result<(), crate::Error> {
                unimplemented!()
            }

            fn await_input(&mut self, _timeout: NextTimeout) -> Result<bool, crate::Error> {
                Err(crate::Error::Timeout(Timeout::Global))
            }

            fn is_open(&mut self) -> bool {
                unimplemented!()
            }
        }

        let mut adapter = TransportAdapter::new(FailingTransport(LazyBuffers::new(1024, 1024)));
        let Err(expected_error) = adapter.read(&mut [0u8; 10]) else {
            panic!("Expected error, but got success");
        };

        assert_eq!(expected_error.kind(), io::ErrorKind::TimedOut);
    }
}
