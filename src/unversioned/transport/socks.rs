use std::fmt;
use std::io::{self, Read, Write};
use std::net::SocketAddr;

use crate::Error;
use crate::proxy::{Proxy, ProxyProtocol};
use crate::util::{AuthorityExt, UriExt};

use super::chain::Either;

use super::tcp::{self, TcpTransport};
use super::time::Instant;
use super::{ConnectionDetails, Connector, LazyBuffers, NextTimeout, ResolvedSocketAddrs};
use super::{Transport, TransportAdapter};

/// Connector for SOCKS proxies.
///
/// Requires the **socks-proxy** feature.
///
/// The connector looks at the proxy settings in [`proxy`](crate::config::ConfigBuilder::proxy) to
/// determine whether to attempt a proxy connection or not.
#[derive(Default)]
pub struct SocksConnector(());

impl<In: Transport> Connector<In> for SocksConnector {
    type Out = Either<In, TcpTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        Self::connect_with(details, chained, |addrs, now, timeout| {
            let stream = tcp::try_connect(
                addrs,
                now,
                timeout,
                details.current_time.clone(),
                details.config,
            )?;
            let buffers = LazyBuffers::new(
                details.config.input_buffer_size(),
                details.config.output_buffer_size(),
            );
            let transport = TcpTransport::new(stream, buffers);
            (details.register_shutdown)(transport.shutdown_handle())?;
            Ok(transport)
        })
    }
}

impl SocksConnector {
    fn connect_with<In: Transport, Out: Transport>(
        details: &ConnectionDetails,
        chained: Option<In>,
        connect: impl FnOnce(&ResolvedSocketAddrs, Instant, NextTimeout) -> Result<Out, Error>,
    ) -> Result<Option<Either<In, Out>>, Error> {
        let proxy = match details.config.proxy() {
            Some(v) if v.protocol().is_socks() => v,
            // If there is no proxy configured, or it isn't a SOCKS proxy, use whatever is chained.
            _ => {
                trace!("SOCKS not configured");
                return Ok(chained.map(Either::A));
            }
        };

        if chained.is_some() {
            trace!("Skip");
            return Ok(chained.map(Either::A));
        }

        // Check if this host is not supposed to be proxied. A direct connection
        // does not need the proxy, so do this before resolving it.
        if proxy.is_no_proxy(details.uri) {
            return Ok(None);
        }

        let target = Target::new(details, proxy)?;

        // Keep all setup steps within the budget that started before this connector.
        let timeout_at = |now: Instant| {
            let mut timeout = details.timeout;
            if !timeout.after.is_not_happening() {
                let elapsed = now.duration_since(details.now);
                timeout.after = timeout.after.saturating_sub(*elapsed).into();
            }
            timeout.check()
        };

        let proxy_addrs = details.resolver.resolve(
            proxy.uri(),
            details.config,
            timeout_at((details.current_time)())?,
        )?;

        // Connect to the proxy like TcpConnector connects to a server, within the
        // connect timeout and trying each resolved address.
        let now = (details.current_time)();
        let transport = connect(&proxy_addrs, now, timeout_at(now)?)?;

        // The adapter deducts elapsed time between handshake reads and writes,
        // while TcpTransport also enforces the per-read and per-write limits.
        let mut w = TransportAdapter::new(transport, details.current_time.clone());
        w.set_timeout(timeout_at((details.current_time)())?);

        trace!("Try connect {} -> {:?}", proxy.protocol(), target);
        handshake(&mut w, proxy, target)?;
        debug!("{} connected -> {:?}", proxy.protocol(), target);

        Ok(Some(Either::B(w.into_inner())))
    }
}

/// The address the proxy is asked to connect to.
#[derive(Debug, Clone, Copy)]
enum Target<'a> {
    Ip(SocketAddr),
    Domain(&'a str, u16),
}

impl<'a> Target<'a> {
    fn new(details: &ConnectionDetails<'a>, proxy: &Proxy) -> Result<Self, Error> {
        if proxy.resolve_target() {
            // The target is already resolved by run().
            let addr = details.addrs.first().ok_or(Error::HostNotFound)?;
            return Ok(Target::Ip(*addr));
        }

        // Do not resolve the target locally, instead pass (host, port)
        // to the proxy and let it resolve. An IP address is passed as one.
        let (_, port) = details.uri.host_port();
        // unwrap is ok because run() checks ensure_valid_url().
        let host = details.uri.authority().unwrap().host_bare();

        match host.parse() {
            Ok(ip) => Ok(Target::Ip(SocketAddr::new(ip, port))),
            Err(_) => Ok(Target::Domain(host, port)),
        }
    }

    fn port(&self) -> u16 {
        match self {
            Target::Ip(addr) => addr.port(),
            Target::Domain(_, port) => *port,
        }
    }
}

fn handshake<S: Read + Write>(w: &mut S, proxy: &Proxy, target: Target) -> io::Result<()> {
    match proxy.protocol() {
        ProxyProtocol::Socks4 | ProxyProtocol::Socks4A => {
            if proxy.username().is_some() {
                debug!("SOCKS4 does not support username/password");
            }

            socks4(w, target)
        }

        ProxyProtocol::Socks5 | ProxyProtocol::Socks5h => socks5(w, proxy, target),

        _ => unreachable!(), // HTTP(s) proxies.
    }
}

/// SOCKS4 CONNECT, and SOCKS4A when the target is a host name. The user id is empty.
fn socks4<S: Read + Write>(w: &mut S, target: Target) -> io::Result<()> {
    // Version, command (CONNECT), port.
    let mut request = vec![4, 1];
    request.extend_from_slice(&target.port().to_be_bytes());

    match target {
        Target::Ip(SocketAddr::V4(addr)) => {
            request.extend_from_slice(&addr.ip().octets());
            request.push(0); // user id
        }
        Target::Ip(SocketAddr::V6(_)) => {
            return Err(invalid_input("SOCKS4 does not support IPv6"));
        }
        Target::Domain(host, _) => {
            // SOCKS4A: the address 0.0.0.1 tells the proxy to resolve the host
            // name sent after the user id.
            request.extend_from_slice(&[0, 0, 0, 1]);
            request.push(0); // user id
            request.extend_from_slice(host.as_bytes());
            request.push(0);
        }
    }

    w.write_all(&request)?;

    // Version (0), status, then a port and address that are not used for CONNECT.
    let mut reply = [0; 8];
    w.read_exact(&mut reply)?;

    if reply[0] != 0 {
        return Err(invalid_data("invalid response version"));
    }

    match reply[1] {
        90 => Ok(()),
        91 => Err(io::Error::other("request rejected or failed")),
        92 => Err(permission_denied(
            "request rejected because SOCKS server cannot connect to identd on the client",
        )),
        93 => Err(permission_denied(
            "request rejected because the client program and identd report different user-ids",
        )),
        _ => Err(invalid_data("invalid response code")),
    }
}

/// SOCKS5 CONNECT (RFC 1928), with username/password authentication (RFC 1929)
/// when the proxy has a username.
fn socks5<S: Read + Write>(w: &mut S, proxy: &Proxy, target: Target) -> io::Result<()> {
    let username = proxy.username();

    // Version, then the offered methods. No authentication (0) is always offered,
    // username/password (2) only when there is a username.
    let greeting: &[u8] = if username.is_some() {
        &[5, 2, 2, 0]
    } else {
        &[5, 1, 0]
    };
    w.write_all(greeting)?;

    // Version, then the method the proxy chose.
    let mut reply = [0; 2];
    w.read_exact(&mut reply)?;

    if reply[0] != 5 {
        return Err(invalid_data("invalid response version"));
    }

    match (reply[1], username) {
        (0, _) => {}
        (2, Some(username)) => {
            let password = proxy.password().unwrap_or("");
            authenticate(w, username, password)?;
        }
        (0xff, _) => return Err(io::Error::other("no acceptable auth methods")),
        _ => return Err(io::Error::other("unknown auth method")),
    }

    // Version, command (CONNECT), reserved, address type, address, port.
    let mut request = vec![5, 1, 0];

    match target {
        Target::Ip(SocketAddr::V4(addr)) => {
            request.push(1);
            request.extend_from_slice(&addr.ip().octets());
        }
        Target::Ip(SocketAddr::V6(addr)) => {
            request.push(4);
            request.extend_from_slice(&addr.ip().octets());
        }
        Target::Domain(host, _) => {
            let len =
                u8::try_from(host.len()).map_err(|_| invalid_input("domain name too long"))?;
            request.push(3);
            request.push(len);
            request.extend_from_slice(host.as_bytes());
        }
    }

    request.extend_from_slice(&target.port().to_be_bytes());
    w.write_all(&request)?;

    // Version, then status.
    w.read_exact(&mut reply)?;

    if reply[0] != 5 {
        return Err(invalid_data("invalid response version"));
    }

    if reply[1] != 0 {
        return Err(socks5_failure(reply[1]));
    }

    // Reserved, then the type of the address the proxy bound. The address and
    // port are not used for CONNECT, but must be read past.
    w.read_exact(&mut reply)?;

    if reply[0] != 0 {
        return Err(invalid_data("invalid reserved byte"));
    }

    let len = match reply[1] {
        1 => 4,
        3 => {
            let mut len = [0];
            w.read_exact(&mut len)?;
            len[0] as usize
        }
        4 => 16,
        _ => return Err(io::Error::other("unsupported address type")),
    };

    let mut bound = [0; u8::MAX as usize + 2];
    w.read_exact(&mut bound[..len + 2])?;

    Ok(())
}

/// The error for a SOCKS5 reply other than succeeded (0).
fn socks5_failure(reply: u8) -> io::Error {
    let reason = match reply {
        1 => "general SOCKS server failure",
        2 => "connection not allowed by ruleset",
        3 => "network unreachable",
        4 => "host unreachable",
        5 => "connection refused",
        6 => "TTL expired",
        7 => "command not supported",
        8 => "address kind not supported",
        _ => "unknown error",
    };

    io::Error::other(reason)
}

fn authenticate<S: Read + Write>(w: &mut S, username: &str, password: &str) -> io::Result<()> {
    // Each is sent after its length in one byte, and must not be empty.
    let valid = 1..=u8::MAX as usize;

    if !valid.contains(&username.len()) {
        return Err(invalid_input("invalid username"));
    }
    if !valid.contains(&password.len()) {
        return Err(invalid_input("invalid password"));
    }

    // Version, username, password.
    let mut request = vec![1];
    request.push(username.len() as u8);
    request.extend_from_slice(username.as_bytes());
    request.push(password.len() as u8);
    request.extend_from_slice(password.as_bytes());
    w.write_all(&request)?;

    // Version, then status.
    let mut reply = [0; 2];
    w.read_exact(&mut reply)?;

    if reply[0] != 1 {
        return Err(invalid_data("invalid response version"));
    }

    if reply[1] != 0 {
        return Err(permission_denied("password authentication failed"));
    }

    Ok(())
}

fn invalid_input(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

fn invalid_data(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

fn permission_denied(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason)
}

impl fmt::Debug for SocksConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SocksConnector").finish()
    }
}

#[cfg(test)]
mod test {
    use std::io::Cursor;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::config::Config;
    use crate::http::Uri;
    use crate::transport::{Buffers, NextTimeout, TcpConnector};
    use crate::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
    use crate::{Agent, Timeout};

    // Resolves IP addresses only, so no test depends on DNS.
    #[derive(Debug)]
    struct IpOnly;

    impl Resolver for IpOnly {
        fn resolve(
            &self,
            uri: &Uri,
            _: &Config,
            _: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, Error> {
            let (host, port) = uri.host_port();
            let ip: IpAddr = host.parse().map_err(|_| Error::HostNotFound)?;
            let mut addrs = self.empty();
            addrs.push(SocketAddr::new(ip, port));
            Ok(addrs)
        }
    }

    // Advance the shared clock to simulate time spent before and during proxy DNS.
    #[derive(Debug)]
    struct TimedProxyResolver {
        elapsed: Arc<Mutex<Duration>>,
        lookup_time: Duration,
        seen: Mutex<Option<NextTimeout>>,
    }

    impl Resolver for TimedProxyResolver {
        fn resolve(
            &self,
            uri: &Uri,
            config: &Config,
            timeout: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, Error> {
            *self.seen.lock().unwrap() = Some(timeout);
            *self.elapsed.lock().unwrap() += self.lookup_time;
            IpOnly.resolve(uri, config, timeout)
        }
    }

    #[derive(Debug, Default)]
    struct TimedIo {
        connects: Vec<NextTimeout>,
        reads: Vec<NextTimeout>,
        writes: Vec<NextTimeout>,
        replies: std::collections::VecDeque<&'static [u8]>,
        sent: Vec<u8>,
        dropped: bool,
    }

    #[derive(Debug)]
    struct TimedConnector {
        io: Arc<Mutex<TimedIo>>,
        elapsed: Arc<Mutex<Duration>>,
    }

    impl TimedConnector {
        fn new(resolver: &TimedProxyResolver) -> Self {
            Self {
                io: Arc::new(Mutex::new(TimedIo::default())),
                elapsed: resolver.elapsed.clone(),
            }
        }
    }

    impl<In: Transport> Connector<In> for TimedConnector {
        type Out = Either<In, TimedTransport>;

        fn connect(
            &self,
            details: &ConnectionDetails,
            chained: Option<In>,
        ) -> Result<Option<Self::Out>, Error> {
            SocksConnector::connect_with(details, chained, |_, _, timeout| {
                self.io.lock().unwrap().connects.push(timeout);
                Ok(TimedTransport {
                    io: self.io.clone(),
                    elapsed: self.elapsed.clone(),
                    buffers: LazyBuffers::new(4096, 4096),
                })
            })
        }
    }

    #[derive(Debug)]
    struct TimedTransport {
        io: Arc<Mutex<TimedIo>>,
        elapsed: Arc<Mutex<Duration>>,
        buffers: LazyBuffers,
    }

    impl Transport for TimedTransport {
        fn buffers(&mut self) -> &mut dyn Buffers {
            &mut self.buffers
        }

        fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
            let timeout = timeout.for_write().check()?;
            let mut io = self.io.lock().unwrap();
            io.writes.push(timeout);
            io.sent.extend_from_slice(&self.buffers.output()[..amount]);
            Ok(())
        }

        fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
            let timeout = timeout.for_read().check()?;
            let mut io = self.io.lock().unwrap();
            io.reads.push(timeout);
            if let Some(reply) = io.replies.pop_front() {
                self.buffers.input_append_buf()[..reply.len()].copy_from_slice(reply);
                self.buffers.input_appended(reply.len());
                return Ok(true);
            }
            // A silent peer consumes its allowance instantly on the simulated clock.
            assert!(
                !timeout.after.is_not_happening(),
                "unscripted read without a timeout"
            );
            *self.elapsed.lock().unwrap() += *timeout.after;
            Err(Error::Timeout(timeout.reason))
        }

        fn is_open(&mut self) -> bool {
            true
        }
    }

    impl Drop for TimedTransport {
        fn drop(&mut self) {
            self.io.lock().unwrap().dropped = true;
        }
    }

    fn connect_with_elapsed(
        proxy: Proxy,
        resolver: &TimedProxyResolver,
        connector: &TimedConnector,
        timeout: NextTimeout,
    ) -> Result<(), Error> {
        let uri: Uri = "http://127.0.0.1:1/".parse().unwrap();
        let config = Agent::config_builder().proxy(Some(proxy)).build();
        let mut addrs = resolver.empty();
        addrs.push("127.0.0.1:1".parse().unwrap());
        let elapsed = resolver.elapsed.clone();
        let start = Instant::now();
        let details = ConnectionDetails {
            uri: &uri,
            addrs,
            config: &config,
            request_level: false,
            resolver,
            now: start,
            timeout,
            current_time: Arc::new(move || start + (*elapsed.lock().unwrap()).into()),
            run_connector: Arc::new(|_| unreachable!()),
            register_shutdown: Arc::new(|_| Ok(())),
        };
        connector.connect(&details, None::<()>).map(|_| ())
    }

    #[test]
    fn proxy_lookup_preserves_remaining_budget() {
        for reason in [Timeout::Global, Timeout::PerCall, Timeout::Connect] {
            let proxy = Proxy::new("socks5h://127.0.0.1:1080").unwrap();
            let resolver = TimedProxyResolver {
                elapsed: Arc::new(Mutex::new(Duration::from_millis(250))),
                lookup_time: Duration::from_millis(500),
                seen: Mutex::new(None),
            };
            let connector = TimedConnector::new(&resolver);
            let timeout = NextTimeout {
                after: Duration::from_secs(1).into(),
                reason,
                per_read: Some(Duration::from_secs(2).into()),
                per_write: Some(Duration::from_secs(3).into()),
            };
            let result = connect_with_elapsed(proxy, &resolver, &connector, timeout);
            assert!(matches!(result, Err(Error::Timeout(actual)) if actual == reason));

            let seen = resolver.seen.lock().unwrap().unwrap();
            assert_eq!(seen.after, Duration::from_millis(750).into());
            assert_eq!(seen.reason, reason);
            assert_eq!(seen.per_read, timeout.per_read);
            assert_eq!(seen.per_write, timeout.per_write);
            let io = connector.io.lock().unwrap();
            assert_eq!(io.connects.len(), 1);
            assert_eq!(io.connects[0].after, Duration::from_millis(250).into());
            assert_eq!(io.reads.len(), 1);
            assert_eq!(io.reads[0].after, Duration::from_millis(250).into());
            assert!(io.dropped, "connection left open");
            assert_eq!(*resolver.elapsed.lock().unwrap(), Duration::from_secs(1));
        }
    }

    #[test]
    fn expired_proxy_lookup_does_not_open_socket() {
        for reason in [Timeout::Global, Timeout::PerCall, Timeout::Connect] {
            let proxy = Proxy::new("socks5h://127.0.0.1:1080").unwrap();
            let resolver = TimedProxyResolver {
                elapsed: Arc::new(Mutex::new(Duration::ZERO)),
                lookup_time: Duration::from_secs(1),
                seen: Mutex::new(None),
            };
            let connector = TimedConnector::new(&resolver);
            let result = connect_with_elapsed(
                proxy,
                &resolver,
                &connector,
                NextTimeout {
                    after: Duration::from_secs(1).into(),
                    reason,
                    per_read: Some(Duration::from_millis(20).into()),
                    ..NextTimeout::default()
                },
            );
            assert!(matches!(result, Err(Error::Timeout(actual)) if actual == reason));
            assert!(connector.io.lock().unwrap().connects.is_empty());
        }
    }

    #[test]
    fn proxy_lookup_does_not_spend_per_read_limit() {
        let proxy = Proxy::new("socks5h://127.0.0.1:1080").unwrap();
        let resolver = TimedProxyResolver {
            elapsed: Arc::new(Mutex::new(Duration::from_millis(250))),
            lookup_time: Duration::from_secs(2),
            seen: Mutex::new(None),
        };
        let connector = TimedConnector::new(&resolver);
        let result = connect_with_elapsed(
            proxy,
            &resolver,
            &connector,
            NextTimeout {
                per_read: Some(Duration::from_millis(100).into()),
                ..NextTimeout::default()
            },
        );
        assert!(matches!(result, Err(Error::Timeout(Timeout::PerRead))));
        let seen = resolver.seen.lock().unwrap().unwrap();
        assert!(seen.after.is_not_happening());
        let io = connector.io.lock().unwrap();
        assert_eq!(io.reads.len(), 1);
        assert_eq!(io.reads[0].after, Duration::from_millis(100).into());
        assert!(io.dropped, "connection left open");
        assert_eq!(
            *resolver.elapsed.lock().unwrap(),
            Duration::from_millis(2350)
        );
    }

    #[cfg(any(feature = "_rustls", feature = "native-tls-no-default"))]
    fn tls_after_socks_preserves_remaining_budget<C>(
        make_tls: impl Fn() -> C,
        tls_config: crate::tls::TlsConfig,
    ) where
        C: Connector<Either<(), TimedTransport>>,
    {
        for reason in [Timeout::Global, Timeout::PerCall, Timeout::Connect] {
            let proxy = Proxy::new("socks5h://127.0.0.1:1080").unwrap();
            let resolver = TimedProxyResolver {
                elapsed: Arc::new(Mutex::new(Duration::ZERO)),
                lookup_time: Duration::from_millis(750),
                seen: Mutex::new(None),
            };
            let connector = TimedConnector::new(&resolver);
            connector
                .io
                .lock()
                .unwrap()
                .replies
                .extend([&[5, 0][..], SUCCEEDED5]);
            let uri: Uri = "https://127.0.0.1/".parse().unwrap();
            let config = Agent::config_builder()
                .proxy(Some(proxy))
                .tls_config(tls_config.clone())
                .build();
            let elapsed = resolver.elapsed.clone();
            let start = Instant::now();
            let details = ConnectionDetails {
                uri: &uri,
                addrs: resolver.empty(),
                config: &config,
                request_level: false,
                resolver: &resolver,
                now: start,
                timeout: NextTimeout {
                    after: Duration::from_secs(1).into(),
                    reason,
                    ..NextTimeout::default()
                },
                current_time: Arc::new(move || start + (*elapsed.lock().unwrap()).into()),
                run_connector: Arc::new(|_| unreachable!()),
                register_shutdown: Arc::new(|_| Ok(())),
            };
            let io = connector.io.clone();
            let result = connector.chain(make_tls()).connect(&details, None::<()>);
            assert!(
                matches!(result, Err(Error::Timeout(actual)) if actual == reason),
                "{reason}: {result:?}"
            );
            let io = io.lock().unwrap();
            assert_eq!(
                io.reads.len(),
                3,
                "two SOCKS replies, then a silent TLS peer"
            );
            for timeout in &io.reads {
                assert_eq!(timeout.after, Duration::from_millis(250).into());
                assert_eq!(timeout.reason, reason);
            }
            assert!(io.sent[13..].starts_with(&[22, 3]), "expected ClientHello");
            assert!(io.dropped, "connection left open");
            assert_eq!(*resolver.elapsed.lock().unwrap(), Duration::from_secs(1));
        }
    }

    #[cfg(feature = "_rustls")]
    #[test]
    fn rustls_after_socks_uses_remaining_budget() {
        tls_after_socks_preserves_remaining_budget(
            crate::transport::RustlsConnector::default,
            crate::tls::TlsConfig::builder()
                .disable_verification(true)
                .unversioned_rustls_crypto_provider(Arc::new(
                    rustls::crypto::aws_lc_rs::default_provider(),
                ))
                .build(),
        );
    }

    #[cfg(feature = "native-tls-no-default")]
    #[test]
    fn native_tls_after_socks_uses_remaining_budget() {
        tls_after_socks_preserves_remaining_budget(
            crate::transport::NativeTlsConnector::default,
            crate::tls::TlsConfig::builder()
                .provider(crate::tls::TlsProvider::NativeTls)
                .disable_verification(true)
                .build(),
        );
    }

    // SOCKS, then plain TCP, without the in-memory transport of the _test feature.
    fn agent(proxy: Proxy) -> Agent {
        let config = Agent::config_builder().proxy(Some(proxy)).build();
        let connector = ().chain(SocksConnector::default()).chain(TcpConnector::default());
        Agent::with_parts(config, connector, IpOnly)
    }

    fn respond_ok(stream: &mut TcpStream) {
        let _ = stream.read(&mut [0; 4096]);
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        stream.write_all(response).unwrap();
    }

    #[test]
    fn no_proxy_goes_direct_without_resolving_proxy() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", server.local_addr().unwrap());
        thread::spawn(move || respond_ok(&mut server.accept().unwrap().0));

        // IpOnly cannot resolve the proxy, like a proxy name that is not in DNS.
        let proxy = Proxy::builder(ProxyProtocol::Socks5)
            .host("proxy.invalid")
            .no_proxy("127.0.0.1")
            .build()
            .unwrap();

        let mut response = agent(proxy).get(&url).call().unwrap();
        assert_eq!(response.body_mut().read_to_string().unwrap(), "ok");
    }

    #[test]
    fn silent_proxy_times_out() {
        for protocol in ["socks4", "socks4a", "socks5", "socks5h"] {
            let proxy = Proxy::new(&format!("{protocol}://127.0.0.1:1080")).unwrap();
            let resolver = TimedProxyResolver {
                elapsed: Arc::new(Mutex::new(Duration::ZERO)),
                lookup_time: Duration::ZERO,
                seen: Mutex::new(None),
            };
            let connector = TimedConnector::new(&resolver);
            let limit = Duration::from_millis(200);
            let result = connect_with_elapsed(
                proxy,
                &resolver,
                &connector,
                NextTimeout {
                    after: limit.into(),
                    reason: Timeout::Global,
                    ..NextTimeout::default()
                },
            );
            assert!(matches!(result, Err(Error::Timeout(Timeout::Global))));
            assert_eq!(*resolver.elapsed.lock().unwrap(), limit);
            assert!(connector.io.lock().unwrap().dropped, "connection left open");
        }
    }

    #[test]
    fn request_through_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("socks5h://{}", listener.local_addr().unwrap());
        let proxy = Proxy::new(&proxy).unwrap();

        // Answers the handshake, then the request as if it were the target.
        let proxy_side = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut greeting = [0; 3];
            stream.read_exact(&mut greeting).unwrap();
            stream.write_all(&[5, 0]).unwrap();
            let mut request = [0; 18];
            stream.read_exact(&mut request).unwrap();
            stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
            respond_ok(&mut stream);
            [&greeting[..], &request[..]].concat()
        });

        let mut response = agent(proxy).get("http://example.com/").call().unwrap();
        assert_eq!(response.body_mut().read_to_string().unwrap(), "ok");

        let sent = proxy_side.join().unwrap();
        let expected = [&[5, 1, 0, 5, 1, 0, 3, 11][..], b"example.com", &[0, 80]].concat();
        assert_eq!(sent, expected);
    }

    // Plays the proxy's side of a handshake from memory.
    struct Script {
        replies: Cursor<Vec<u8>>,
        sent: Vec<u8>,
    }

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            // Force fragmented I/O even when a whole reply is available.
            let len = buf.len().min(1);
            self.replies.read(&mut buf[..len])
        }
    }

    impl Write for Script {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let len = buf.len().min(1);
            self.sent.extend_from_slice(&buf[..len]);
            Ok(len)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    // Bytes in parts, so that text can be written as text.
    type Parts = &'static [&'static [u8]];

    fn script(proxy: &str, target: Target, replies: Parts) -> (io::Result<()>, Script) {
        let proxy = Proxy::new(proxy).unwrap();
        let mut script = Script {
            replies: Cursor::new(replies.concat()),
            sent: vec![],
        };
        let result = handshake(&mut script, &proxy, target);
        (result, script)
    }

    const IPV4: Target = Target::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 80));
    const IPV6: Target = Target::Ip(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 80));
    const DOMAIN: Target = Target::Domain("example.com", 443);

    const GRANTED4: &[u8] = &[0, 90, 0, 0, 0, 0, 0, 0];
    const SUCCEEDED5: &[u8] = &[5, 0, 0, 1, 0, 0, 0, 0, 0, 0];

    #[derive(Debug)]
    struct ReadAheadTransport {
        buffers: LazyBuffers,
        replies: Cursor<Vec<u8>>,
        reads: usize,
    }

    impl Transport for ReadAheadTransport {
        fn buffers(&mut self) -> &mut dyn Buffers {
            &mut self.buffers
        }

        fn transmit_output(&mut self, _: usize, timeout: NextTimeout) -> Result<(), Error> {
            timeout.for_write().check()?;
            Ok(())
        }

        fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
            timeout.for_read().check()?;
            self.reads += 1;
            let len = self.replies.read(self.buffers.input_append_buf())?;
            self.buffers.input_appended(len);
            Ok(len > 0)
        }

        fn is_open(&mut self) -> bool {
            true
        }
    }

    // rust-socks #17 lost application data read alongside the SOCKS reply when
    // its temporary BufReader was dropped. Exercise actual adapter read-ahead
    // and retain the unread data when handing the transport back to the caller.
    // https://github.com/sfackler/rust-socks/pull/17
    #[test]
    fn handshake_read_ahead_preserves_application_data() {
        let application = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        for (proxy, target, replies) in [
            ("socks4://proxy", IPV4, GRANTED4.to_vec()),
            ("socks4a://proxy", DOMAIN, GRANTED4.to_vec()),
            ("socks5://proxy", IPV4, [&[5, 0][..], SUCCEEDED5].concat()),
            (
                "socks5h://proxy",
                DOMAIN,
                [&[5, 0, 5, 0, 0, 3, 4][..], b"host", &[0, 0]].concat(),
            ),
            (
                "socks5://user:pass@proxy",
                IPV6,
                [&[5, 2, 1, 0, 5, 0, 0, 4][..], &[0; 18]].concat(),
            ),
        ] {
            let proxy = Proxy::new(proxy).unwrap();
            let transport = ReadAheadTransport {
                buffers: LazyBuffers::new(512, 512),
                replies: Cursor::new([replies, application.to_vec()].concat()),
                reads: 0,
            };
            let now = Instant::now();
            let current_time = Arc::new(move || now);
            let mut adapter = TransportAdapter::new(transport, current_time.clone());
            handshake(&mut adapter, &proxy, target).unwrap();

            let transport = adapter.into_inner();
            assert_eq!(
                transport.reads, 1,
                "reply and application data must be read together"
            );
            assert_eq!(transport.buffers.input(), application);

            let mut adapter = TransportAdapter::new(transport, current_time);
            let mut received = vec![0; application.len()];
            adapter.read_exact(&mut received).unwrap();
            assert_eq!(received, application);
            assert_eq!(
                adapter.into_inner().reads,
                1,
                "use the retained input buffer"
            );
        }
    }

    #[test]
    fn handshakes() {
        let cases: &[(&str, Target, Parts, Parts)] = &[
            (
                "socks4://proxy",
                IPV4,
                &[&[4, 1, 0, 80, 10, 0, 0, 1, 0]],
                &[GRANTED4],
            ),
            (
                "socks4a://proxy",
                DOMAIN,
                &[&[4, 1, 1, 187, 0, 0, 0, 1, 0], b"example.com", &[0]],
                &[GRANTED4],
            ),
            (
                // ureq has always sent an empty SOCKS4 user id.
                "socks4://user:pass@proxy",
                IPV4,
                &[&[4, 1, 0, 80, 10, 0, 0, 1, 0]],
                &[GRANTED4],
            ),
            (
                "socks5://proxy",
                IPV4,
                &[&[5, 1, 0], &[5, 1, 0, 1, 10, 0, 0, 1, 0, 80]],
                &[&[5, 0], SUCCEEDED5],
            ),
            (
                // The proxy names the bound address by host name.
                "socks5h://proxy",
                DOMAIN,
                &[&[5, 1, 0], &[5, 1, 0, 3, 11], b"example.com", &[1, 187]],
                &[&[5, 0], &[5, 0, 0, 3, 4], b"host", &[0, 0]],
            ),
            (
                "socks5://user:pass@proxy",
                IPV6,
                &[
                    &[5, 2, 2, 0],
                    &[1, 4],
                    b"user",
                    &[4],
                    b"pass",
                    &[5, 1, 0, 4],
                    &[0; 15], // ::1
                    &[1, 0, 80],
                ],
                &[&[5, 2], &[1, 0], &[5, 0, 0, 4], &[0; 18]],
            ),
            (
                // The proxy may choose no authentication even when offered a password.
                "socks5://user:pass@proxy",
                IPV4,
                &[&[5, 2, 2, 0], &[5, 1, 0, 1, 10, 0, 0, 1, 0, 80]],
                &[&[5, 0], SUCCEEDED5],
            ),
        ];

        for (proxy, target, sent, replies) in cases {
            let proxy_config = Proxy::new(proxy).unwrap();
            let mut script = Script {
                replies: Cursor::new([replies.concat(), b"application data".to_vec()].concat()),
                sent: vec![],
            };
            let result = handshake(&mut script, &proxy_config, *target);
            result.unwrap_or_else(|e| panic!("{proxy}: {e}"));
            assert_eq!(script.sent, sent.concat(), "{proxy}");

            // Consume exactly the reply, leaving application bytes for the caller.
            let mut remaining = Vec::new();
            script.read_to_end(&mut remaining).unwrap();
            assert_eq!(remaining, b"application data", "{proxy}");
        }
    }

    #[test]
    fn handshake_failures() {
        use io::ErrorKind::{InvalidInput, Other, PermissionDenied, UnexpectedEof};

        let cases: &[(&str, Target, Parts, io::ErrorKind, &str)] = &[
            (
                "socks4://proxy",
                IPV4,
                &[&[0, 91, 0, 0, 0, 0, 0, 0]],
                Other,
                "request rejected or failed",
            ),
            (
                "socks4://proxy",
                IPV6,
                &[],
                InvalidInput,
                "SOCKS4 does not support IPv6",
            ),
            (
                "socks5://proxy",
                IPV4,
                &[&[5, 0xff]],
                Other,
                "no acceptable auth methods",
            ),
            (
                // A password was not offered, so the proxy cannot ask for one.
                "socks5://proxy",
                IPV4,
                &[&[5, 2]],
                Other,
                "unknown auth method",
            ),
            (
                "socks5://user:pass@proxy",
                IPV4,
                &[&[5, 2], &[1, 1]],
                PermissionDenied,
                "password authentication failed",
            ),
            (
                "socks5://proxy",
                IPV4,
                &[&[5, 0], &[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]],
                Other,
                "connection refused",
            ),
        ];

        for (proxy, target, replies, kind, reason) in cases {
            let (result, _) = script(proxy, *target, replies);
            let error = result.unwrap_err();
            assert_eq!(error.kind(), *kind, "{proxy}: {error}");
            assert_eq!(error.to_string(), *reason, "{proxy}");
        }

        // A reply cut short is an error, not a connection.
        let (result, _) = script("socks5://proxy", IPV4, &[&[5, 0], &[5, 0, 0, 1, 0, 0]]);
        assert_eq!(result.unwrap_err().kind(), UnexpectedEof);
    }

    #[test]
    fn domain_name_length() {
        let proxy = Proxy::new("socks5h://proxy").unwrap();
        for len in [1, 254, 255, 256] {
            let host = "x".repeat(len);
            let mut script = Script {
                replies: Cursor::new([&[5, 0][..], SUCCEEDED5].concat()),
                sent: vec![],
            };
            let result = handshake(&mut script, &proxy, Target::Domain(&host, 443));
            if len == 256 {
                let error = result.unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
                assert_eq!(error.to_string(), "domain name too long");
                assert_eq!(script.sent, [5, 1, 0]);
            } else {
                result.unwrap();
                assert_eq!(
                    script.sent,
                    [
                        &[5, 1, 0, 5, 1, 0, 3, len as u8][..],
                        host.as_bytes(),
                        &[1, 187]
                    ]
                    .concat()
                );
            }
        }
    }

    #[test]
    fn every_truncated_reply_is_an_error() {
        let domain_reply = [&[5, 0, 0, 3, 255][..], &[b'x'; 255], &[0, 0]].concat();
        let ipv6_reply = [&[5, 0, 0, 4][..], &[0; 18]].concat();
        for (proxy, reply) in [
            ("socks4://proxy", GRANTED4.to_vec()),
            ("socks4a://proxy", GRANTED4.to_vec()),
            ("socks5://proxy", [&[5, 0][..], SUCCEEDED5].concat()),
            ("socks5://proxy", [&[5, 0][..], &domain_reply].concat()),
            ("socks5://proxy", [&[5, 0][..], &ipv6_reply].concat()),
            (
                "socks5://user:pass@proxy",
                [&[5, 2, 1, 0][..], SUCCEEDED5].concat(),
            ),
        ] {
            let proxy = Proxy::new(proxy).unwrap();
            for len in 0..=reply.len() {
                let mut script = Script {
                    replies: Cursor::new(reply[..len].to_vec()),
                    sent: vec![],
                };
                let result = handshake(&mut script, &proxy, IPV4);
                if len == reply.len() {
                    result.unwrap();
                    assert_eq!(script.replies.position(), len as u64);
                } else {
                    let error = result.unwrap_err();
                    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof, "prefix {len}");
                }
            }
        }
    }

    #[test]
    fn malformed_handshake_replies() {
        use io::ErrorKind::{InvalidData, Other};
        let cases: &[(&str, Parts, io::ErrorKind, &str)] = &[
            (
                "socks4://proxy",
                &[&[4, 90, 0, 0, 0, 0, 0, 0]],
                InvalidData,
                "invalid response version",
            ),
            (
                "socks4://proxy",
                &[&[0, 94, 0, 0, 0, 0, 0, 0]],
                InvalidData,
                "invalid response code",
            ),
            (
                "socks5://proxy",
                &[&[4, 0]],
                InvalidData,
                "invalid response version",
            ),
            (
                "socks5://proxy",
                &[&[5, 0], &[4, 0]],
                InvalidData,
                "invalid response version",
            ),
            (
                "socks5://proxy",
                &[&[5, 0], &[5, 0, 1, 1]],
                InvalidData,
                "invalid reserved byte",
            ),
            (
                "socks5://proxy",
                &[&[5, 0], &[5, 0, 0, 2]],
                Other,
                "unsupported address type",
            ),
            (
                "socks5://user:pass@proxy",
                &[&[5, 2], &[2, 0]],
                InvalidData,
                "invalid response version",
            ),
        ];
        for (proxy, replies, kind, reason) in cases {
            let (result, _) = script(proxy, IPV4, replies);
            let error = result.unwrap_err();
            assert_eq!(error.kind(), *kind, "{proxy}: {error}");
            assert_eq!(error.to_string(), *reason, "{proxy}");
        }
    }

    #[test]
    fn proxy_failure_codes() {
        for (code, reason) in [
            (1, "general SOCKS server failure"),
            (2, "connection not allowed by ruleset"),
            (3, "network unreachable"),
            (4, "host unreachable"),
            (5, "connection refused"),
            (6, "TTL expired"),
            (7, "command not supported"),
            (8, "address kind not supported"),
            (255, "unknown error"),
        ] {
            let mut script = Script {
                replies: Cursor::new(vec![5, 0, 5, code]),
                sent: vec![],
            };
            let proxy = Proxy::new("socks5://proxy").unwrap();
            let error = handshake(&mut script, &proxy, IPV4).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Other);
            assert_eq!(error.to_string(), reason);
        }
        for code in [92, 93] {
            let mut script = Script {
                replies: Cursor::new(vec![0, code, 0, 0, 0, 0, 0, 0]),
                sent: vec![],
            };
            let proxy = Proxy::new("socks4://proxy").unwrap();
            let error = handshake(&mut script, &proxy, IPV4).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        }
    }
}
