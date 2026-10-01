use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::{fmt, io, time};

use crate::config::Config;
use crate::unversioned::transport::time::Instant;
use crate::util::IoResultExt;
use crate::{Error, Timeout};

use super::ResolvedSocketAddrs;
use super::chain::Either;

use super::time::Duration;
use super::{Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport};

#[derive(Default)]
/// Connector for regular TCP sockets.
pub struct TcpConnector(());

impl<In: Transport> Connector<In> for TcpConnector {
    type Out = Either<In, TcpTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        if chained.is_some() {
            // The chained connection overrides whatever we were to open here.
            // In the DefaultConnector chain this would be a SOCKS proxy connection.
            trace!("Skip");
            return Ok(chained.map(Either::A));
        }

        let config = &details.config;
        let stream = try_connect(
            &details.addrs,
            details.now,
            details.timeout,
            details.current_time.clone(),
            config,
        )?;

        let buffers = LazyBuffers::new(config.input_buffer_size(), config.output_buffer_size());
        let transport = TcpTransport::new(stream, buffers);

        Ok(Some(Either::B(transport)))
    }
}

fn try_connect(
    addrs: &ResolvedSocketAddrs,
    start: Instant,
    timeout: NextTimeout,
    current_time: Arc<dyn Fn() -> Instant + Send + Sync + 'static>,
    config: &Config,
) -> Result<TcpStream, Error> {
    try_connect_with(addrs, start, timeout, current_time, |addr, per_addr| {
        try_connect_single(addr, per_addr, config)
    })
}

fn try_connect_with<T>(
    addrs: &ResolvedSocketAddrs,
    start: Instant,
    timeout: NextTimeout,
    current_time: Arc<dyn Fn() -> Instant + Send + Sync + 'static>,
    mut connect: impl FnMut(SocketAddr, Option<Duration>) -> Result<T, Error>,
) -> Result<T, Error> {
    // The idea here is to give each attempt a budget of the total time to try.
    // For a host returning multiple addresses, we share the budget between them
    // using a geometric series that sums to exactly the total budget.
    //
    // Background: https://curl.se/mail/lib-2021-01/0037.html
    //
    // Example: Timeout is 10 seconds, and the host returns 4 addresses.
    //
    // Address 0: 5.33 seconds (53.3% of budget)
    // Address 1: 2.67 seconds (26.7% of budget)
    // Address 2: 1.33 seconds (13.3% of budget)
    // Address 3: 0.67 seconds (6.7% of budget)
    // Sum: 10.0 seconds
    //
    // For a single address, it gets the full budget (100%).
    // We cap the lowest to 10ms.
    //
    const MIN_PER_ADDRESS_TIMEOUT: Duration = Duration::from_millis(10);

    let num_addrs = addrs.len();

    // Pre-calculate the total weight for the geometric series.
    // For weights [1, 1/2, 1/4, 1/8, ...], the sum is 2 * (1 - 1/2^n)
    let total_weight = 2.0 * (1.0 - 0.5_f64.powi(num_addrs as i32));

    // Start with weight 1.0 for the first address, then halve for each subsequent.
    let mut weight = 1.0_f64;

    // The most recent per-address failure, so that a host whose every address
    // fails reports what actually happened instead of a synthesized refusal.
    let mut last_err: Option<Error> = None;

    for addr in addrs {
        // Calculate this address's timeout using geometric series.
        let per_addr = timeout.not_zero().map(|t| {
            let secs = t.as_secs_f64() * weight / total_weight;
            let timeout = Duration::from_millis((secs * 1000.0) as u64);
            timeout.max(MIN_PER_ADDRESS_TIMEOUT)
        });

        match connect(*addr, per_addr) {
            // First that connects
            Ok(v) => return Ok(v),
            // Intercept errors that concern only this address to try next addrs
            Err(Error::Io(e)) if is_addr_specific_error(&e) => {
                trace!("{} failed: {}", addr, e);
                last_err = Some(Error::Io(e));
                continue;
            }
            Err(e @ Error::Timeout(_)) => {
                // Check if we hit the overall global timeout for the connect.
                let elapsed = current_time().duration_since(start);
                if elapsed > timeout.after {
                    return Err(Error::Timeout(timeout.reason));
                }

                // We still got time to try the next address.
                last_err = Some(e);
            }
            // Other errors bail
            Err(e) => return Err(e),
        }

        // Halve the weight for the next address
        weight /= 2.0;
    }

    debug!("Failed to connect to any resolved address");
    Err(last_err.unwrap_or_else(|| {
        Error::Io(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "Connection refused",
        ))
    }))
}

/// Whether a failed connect concerns only the address tried, meaning the next
/// resolved address might still succeed.
///
/// `ConnectionRefused` means this address answered and said no. The
/// unreachable/unavailable kinds mean the local network stack rejected this
/// address at routing level without asking anything: the typical case is a
/// host whose resolver returns IPv6 addresses first but which has no IPv6
/// route, where the AAAA connect fails instantly while the A record would
/// have worked (#1184). Browsers and curl mask that condition by moving on to
/// the next address, which is the behavior matched here.
fn is_addr_specific_error(e: &io::Error) -> bool {
    // On Windows, a VPN or firewall can surface the blocked address family as
    // WSAEACCES, which maps to ErrorKind::PermissionDenied. Match the raw OS
    // error to retry this socket error without retrying every permission error (#1184).
    #[cfg(windows)]
    const WSAEACCES: i32 = 10013;
    #[cfg(windows)]
    if e.raw_os_error() == Some(WSAEACCES) {
        return true;
    }

    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::AddrNotAvailable
    )
}

fn try_connect_single(
    addr: SocketAddr,
    per_addr: Option<Duration>,
    config: &Config,
) -> Result<TcpStream, Error> {
    trace!("Try connect TcpStream to {}", addr);

    let maybe_stream = if let Some(when) = per_addr {
        TcpStream::connect_timeout(&addr, *when)
    } else {
        TcpStream::connect(addr)
    }
    .normalize_would_block();

    let stream = match maybe_stream {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::TimedOut => {
            // The parent replaces this reason if the overall deadline has expired.
            return Err(Error::Timeout(Timeout::Connect));
        }
        Err(e) => return Err(e.into()),
    };

    if config.no_delay() {
        stream.set_nodelay(true)?;
    }

    debug!("Connected TcpStream to {}", addr);

    Ok(stream)
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
        let timeout = timeout.for_write().check()?;
        maybe_update_timeout(
            timeout,
            &mut self.timeout_write,
            &self.stream,
            TcpStream::set_write_timeout,
        )?;

        let output = &self.buffers.output()[..amount];
        match self.stream.write_all(output).normalize_would_block() {
            Ok(v) => Ok(v),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Err(Error::Timeout(timeout.reason)),
            Err(e) => Err(e.into()),
        }?;

        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        let timeout = timeout.for_read().check()?;
        // Proceed to fill the buffers from the TcpStream
        maybe_update_timeout(
            timeout,
            &mut self.timeout_read,
            &self.stream,
            TcpStream::set_read_timeout,
        )?;

        let input = self.buffers.input_append_buf();
        let started = time::Instant::now();
        let result = read_retrying_interrupts(
            &mut self.stream,
            input,
            timeout.not_zero().map(|t| *t),
            || started.elapsed(),
            |stream, left| {
                stream.set_read_timeout(Some(left))?;
                // Keep the cache in step so maybe_update_timeout() later restores the full value.
                self.timeout_read = Some(left.into());
                Ok(())
            },
        );
        let amount = match result.normalize_would_block() {
            Ok(v) => Ok(v),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Err(Error::Timeout(timeout.reason)),
            Err(e) => Err(e.into()),
        }?;
        self.buffers.input_appended(amount);

        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        probe_tcp_stream(&mut self.stream).unwrap_or(false)
    }
}

/// Retry reads interrupted by a signal (EINTR) when a timeout applies.
///
/// Linux does not restart socket reads with SO_RCVTIMEO, even with SA_RESTART.
/// Retries use the remaining timeout; exhausted budgets return `TimedOut`.
/// Without a timeout, preserve `Interrupted` for intentional signal interruptions.
/// See <https://github.com/algesten/ureq/pull/1205> for the signal details.
fn read_retrying_interrupts<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    timeout: Option<time::Duration>,
    mut elapsed: impl FnMut() -> time::Duration,
    mut set_timeout: impl FnMut(&mut R, time::Duration) -> io::Result<()>,
) -> io::Result<usize> {
    loop {
        match reader.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                let Some(timeout) = timeout else {
                    return Err(e);
                };
                let left = timeout.saturating_sub(elapsed());
                if left.is_zero() {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                set_timeout(reader, left)?;
            }
            result => return result,
        }
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
        Ok(_) => {
            debug!("Unexpected bytes from server. Closing connection");
            return Ok(false);
        }
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

#[cfg(test)]
mod test {
    use super::*;

    // Script connect outcomes so routing failures and timeouts are deterministic.
    fn scripted_connect(
        outcomes: Vec<Result<(), Error>>,
        elapsed: Duration,
    ) -> (Result<(), Error>, usize) {
        let mut addrs = ResolvedSocketAddrs::from_fn(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        for i in 0..outcomes.len() {
            addrs.push(SocketAddr::from(([127, 0, 0, 1], 10000 + i as u16)));
        }
        let start = Instant::now();
        let mut outcomes = outcomes.into_iter();
        let mut attempts = 0;
        let result = try_connect_with(
            &addrs,
            start,
            NextTimeout {
                after: Duration::from_secs(10),
                reason: Timeout::Global,
                ..NextTimeout::default()
            },
            Arc::new(move || start + elapsed),
            |addr, _| {
                assert_eq!(addr, addrs[attempts]);
                attempts += 1;
                outcomes.next().unwrap()
            },
        );
        (result, attempts)
    }

    fn io_error(kind: io::ErrorKind) -> Result<(), Error> {
        Err(io::Error::from(kind).into())
    }

    #[test]
    fn timeout_replaces_earlier_unreachable_error() {
        let (result, attempts) = scripted_connect(
            vec![
                io_error(io::ErrorKind::NetworkUnreachable),
                Err(Error::Timeout(Timeout::Connect)),
            ],
            Duration::from_secs(6),
        );
        assert_eq!(attempts, 2);
        assert!(matches!(result, Err(Error::Timeout(Timeout::Connect))));
    }

    #[test]
    fn addr_specific_failures_and_timeout_fall_back_to_success() {
        for failure in [
            io_error(io::ErrorKind::ConnectionRefused),
            io_error(io::ErrorKind::HostUnreachable),
            io_error(io::ErrorKind::NetworkUnreachable),
            io_error(io::ErrorKind::AddrNotAvailable),
            Err(Error::Timeout(Timeout::Connect)),
        ] {
            let (result, attempts) = scripted_connect(
                vec![failure, Ok(()), io_error(io::ErrorKind::PermissionDenied)],
                Duration::from_secs(1),
            );
            assert!(result.is_ok());
            assert_eq!(attempts, 2);
        }
    }

    #[test]
    fn last_io_error_replaces_timeout_and_preserves_details() {
        let (result, attempts) = scripted_connect(
            vec![
                Err(Error::Timeout(Timeout::Connect)),
                Err(io::Error::new(io::ErrorKind::HostUnreachable, "last address").into()),
            ],
            Duration::from_secs(1),
        );
        assert_eq!(attempts, 2);
        let Err(Error::Io(error)) = result else {
            panic!("expected last I/O error: {result:?}");
        };
        assert_eq!(error.kind(), io::ErrorKind::HostUnreachable);
        assert_eq!(error.to_string(), "last address");
    }

    #[test]
    fn all_timeouts_report_connect_timeout() {
        let (result, attempts) = scripted_connect(
            vec![
                Err(Error::Timeout(Timeout::Connect)),
                Err(Error::Timeout(Timeout::Connect)),
            ],
            Duration::from_secs(6),
        );
        assert_eq!(attempts, 2);
        assert!(matches!(result, Err(Error::Timeout(Timeout::Connect))));
    }

    #[test]
    fn overall_timeout_takes_precedence_and_stops_attempts() {
        let (result, attempts) = scripted_connect(
            vec![
                io_error(io::ErrorKind::NetworkUnreachable),
                Err(Error::Timeout(Timeout::Connect)),
                Ok(()),
            ],
            Duration::from_secs(11),
        );
        assert_eq!(attempts, 2);
        assert!(matches!(result, Err(Error::Timeout(Timeout::Global))));
    }

    #[test]
    fn other_io_errors_stop_attempts() {
        let (result, attempts) = scripted_connect(
            vec![io_error(io::ErrorKind::PermissionDenied), Ok(())],
            Duration::from_secs(1),
        );
        assert_eq!(attempts, 1);
        assert!(matches!(result, Err(Error::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied));
    }

    #[test]
    fn empty_address_list_reports_refusal() {
        let (result, attempts) = scripted_connect(vec![], Duration::from_secs(0));
        assert_eq!(attempts, 0);
        assert!(
            matches!(result, Err(Error::Io(e)) if e.kind() == io::ErrorKind::ConnectionRefused)
        );
    }

    #[test]
    fn addr_specific_errors_try_the_next_addr() {
        for kind in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkUnreachable,
            io::ErrorKind::AddrNotAvailable,
        ] {
            assert!(
                is_addr_specific_error(&io::Error::from(kind)),
                "{kind:?} should move on to the next address"
            );
        }
        for kind in [io::ErrorKind::TimedOut, io::ErrorKind::PermissionDenied] {
            assert!(
                !is_addr_specific_error(&io::Error::from(kind)),
                "{kind:?} should bail"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn wsaeacces_tries_the_next_addr() {
        assert!(is_addr_specific_error(&io::Error::from_raw_os_error(10013)));
    }

    // Script read outcomes and elapsed times, returning what the caller saw and every
    // timeout it was given.
    fn scripted_read(
        outcomes: Vec<io::Result<usize>>,
        timeout: Option<time::Duration>,
        elapsed: Vec<time::Duration>,
    ) -> (io::Result<usize>, Vec<time::Duration>) {
        struct Script(std::vec::IntoIter<io::Result<usize>>);
        impl Read for Script {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                self.0.next().unwrap()
            }
        }
        let mut elapsed = elapsed.into_iter();
        let mut timeouts = vec![];
        let result = read_retrying_interrupts(
            &mut Script(outcomes.into_iter()),
            &mut [0; 8],
            timeout,
            || elapsed.next().unwrap(),
            |_, left| {
                timeouts.push(left);
                Ok(())
            },
        );
        (result, timeouts)
    }

    fn interrupted() -> io::Result<usize> {
        Err(io::ErrorKind::Interrupted.into())
    }

    fn secs(s: u64) -> time::Duration {
        time::Duration::from_secs(s)
    }

    #[test]
    fn interrupted_read_is_retried_with_remaining_timeout() {
        let (result, timeouts) = scripted_read(
            vec![interrupted(), interrupted(), Ok(3)],
            Some(secs(10)),
            vec![secs(3), secs(7)],
        );
        assert_eq!(result.unwrap(), 3);
        assert_eq!(timeouts, vec![secs(7), secs(3)]);
    }

    #[test]
    fn interrupted_read_past_timeout_times_out() {
        let (result, timeouts) =
            scripted_read(vec![interrupted(), Ok(3)], Some(secs(1)), vec![secs(1)]);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(timeouts.is_empty());
    }

    #[test]
    fn only_interrupted_reads_are_retried() {
        let (result, timeouts) = scripted_read(
            vec![
                interrupted(),
                Err(io::ErrorKind::ConnectionReset.into()),
                Ok(3),
            ],
            Some(secs(10)),
            vec![secs(1)],
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionReset);
        assert_eq!(timeouts, vec![secs(9)]);
    }

    // Without a timeout, only a handler installed without SA_RESTART causes EINTR. It asked
    // for the interruption, so the read must not be retried.
    #[test]
    fn interrupted_read_without_timeout_is_not_retried() {
        let (result, timeouts) = scripted_read(vec![interrupted(), Ok(3)], None, vec![]);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(timeouts.is_empty());
    }

    #[test]
    fn socket_read_stall_reports_per_read() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        let mut transport = TcpTransport::new(stream, LazyBuffers::new(1024, 1024));
        let timeout = NextTimeout {
            per_read: Some(Duration::from_millis(20)),
            per_write: Some(Duration::from_millis(30)),
            ..NextTimeout::default()
        };
        let error = transport.await_input(timeout).unwrap_err();
        assert!(
            matches!(error, Error::Timeout(Timeout::PerRead)),
            "{error:?}"
        );
        transport.buffers().output()[0] = b'x';
        transport.transmit_output(1, timeout).unwrap();
        assert_eq!(
            transport.stream.write_timeout().unwrap(),
            Some(std::time::Duration::from_millis(30))
        );
    }
}
