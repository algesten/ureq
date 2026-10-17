use std::io;
use std::sync::Arc;

use crate::Timeout;

use super::time::{Duration, Instant};
use super::{NextTimeout, Transport};

/// Helper to turn a [`Transport`] into a std::io [`Read`](io::Read) and [`Write`](io::Write).
///
/// This is useful when integrating with components that expect a regular `Read`/`Write`. In
/// ureq this is used both for the [`RustlsConnector`](crate::unversioned::transport::RustlsConnector) and the
/// [`NativeTlsConnector`](crate::unversioned::transport::NativeTlsConnector).
pub struct TransportAdapter<T: Transport = Box<dyn Transport>> {
    timeout: NextTimeout,
    timeout_set_at: Instant,
    current_time: Arc<dyn Fn() -> Instant + Send + Sync + 'static>,
    transport: T,
}

impl<T: Transport> TransportAdapter<T> {
    /// Creates a new adapter using the provided clock.
    pub fn new(
        transport: T,
        current_time: Arc<dyn Fn() -> Instant + Send + Sync + 'static>,
    ) -> Self {
        Self {
            timeout: NextTimeout {
                after: Duration::NotHappening,
                reason: Timeout::Global,
                ..NextTimeout::default()
            },
            timeout_set_at: current_time(),
            current_time,
            transport,
        }
    }

    /// Set a new value of the timeout.
    pub fn set_timeout(&mut self, timeout: NextTimeout) {
        self.set_timeout_at(timeout, (self.current_time)());
    }

    /// Set a timeout measured from an earlier stage of the connection setup.
    pub(crate) fn set_timeout_at(&mut self, timeout: NextTimeout, started: Instant) {
        self.timeout = timeout;
        self.timeout_set_at = started;
    }

    fn timeout_now(&self) -> NextTimeout {
        let mut timeout = self.timeout;
        timeout.after = remaining_after(timeout.after, self.timeout_set_at, (self.current_time)());
        timeout
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

fn remaining_after(after: Duration, started: Instant, now: Instant) -> Duration {
    if after.is_not_happening() {
        return after;
    }
    match (after, now.duration_since(started)) {
        (Duration::Exact(limit), Duration::Exact(elapsed)) => {
            Duration::Exact(limit.saturating_sub(elapsed))
        }
        (Duration::Exact(_), Duration::NotHappening) => Duration::from_secs(0),
        (Duration::NotHappening, _) => Duration::NotHappening,
    }
}

impl<T: Transport> io::Read for TransportAdapter<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.transport
            .maybe_await_input(self.timeout_now())
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
            .transmit_output(max, self.timeout_now())
            .map_err(|e| e.into_io())?;

        Ok(max)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Error;
    use crate::transport::Buffers;
    use crate::unversioned::transport::LazyBuffers;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct RecordingTransport {
        buffers: LazyBuffers,
        seen: Vec<Duration>,
    }

    impl Transport for RecordingTransport {
        fn buffers(&mut self) -> &mut dyn Buffers {
            &mut self.buffers
        }

        fn transmit_output(&mut self, _: usize, timeout: NextTimeout) -> Result<(), Error> {
            let timeout = timeout.check()?;
            self.seen.push(timeout.after);
            Ok(())
        }

        fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
            let timeout = timeout.check()?;
            self.seen.push(timeout.after);
            let buf = self.buffers.input_append_buf();
            buf[0] = 1;
            self.buffers.input_appended(1);
            Ok(true)
        }

        fn is_open(&mut self) -> bool {
            true
        }
    }

    #[test]
    fn adapter_io_uses_remaining_timeout_budget() {
        use std::io::{Read, Write};

        let clock = Arc::new(Mutex::new(Instant::now()));
        let current_time = {
            let clock = Arc::clone(&clock);
            Arc::new(move || *clock.lock().unwrap())
        };
        let mut adapter = TransportAdapter::new(
            RecordingTransport {
                buffers: LazyBuffers::new(16, 16),
                seen: Vec::new(),
            },
            current_time,
        );
        adapter.set_timeout(NextTimeout {
            after: Duration::from_secs(2),
            reason: Timeout::Connect,
            ..NextTimeout::default()
        });

        adapter.write_all(&[1]).unwrap();
        {
            let mut now = *clock.lock().unwrap();
            now = now + Duration::from_millis(750);
            *clock.lock().unwrap() = now;
        }
        adapter.read_exact(&mut [0]).unwrap();
        {
            let mut now = *clock.lock().unwrap();
            now = now + Duration::from_millis(750);
            *clock.lock().unwrap() = now;
        }
        adapter.write_all(&[1]).unwrap();

        let seen = adapter.into_inner().seen;
        assert_eq!(seen[0], Duration::from_secs(2));
        assert_eq!(seen[1], Duration::from_millis(1250));
        assert_eq!(seen[2], Duration::from_millis(500));
    }

    #[test]
    fn adapter_io_times_out_when_the_budget_is_spent() {
        use std::io::Read;

        let start = Instant::now();
        let clock = Arc::new(Mutex::new(start));
        let current_time = {
            let clock = Arc::clone(&clock);
            Arc::new(move || *clock.lock().unwrap())
        };
        let mut adapter = TransportAdapter::new(
            RecordingTransport {
                buffers: LazyBuffers::new(16, 16),
                seen: Vec::new(),
            },
            current_time,
        );
        adapter.set_timeout(NextTimeout {
            after: Duration::from_secs(1),
            reason: Timeout::Connect,
            ..NextTimeout::default()
        });
        *clock.lock().unwrap() = start + Duration::from_secs(1);

        let error = adapter.read_exact(&mut [0]).unwrap_err();
        let inner = error
            .get_ref()
            .and_then(|e| e.downcast_ref::<Error>())
            .expect("ureq timeout");
        assert!(matches!(inner, Error::Timeout(Timeout::Connect)));
    }
}
