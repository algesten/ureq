use std::fmt;
use std::io::{self, Read, Write};
use std::net::SocketAddr;

use crate::Error;
use crate::proxy::{Proxy, ProxyProtocol};
use crate::util::{AuthorityExt, UriExt};

use super::chain::Either;

use super::tcp::{self, TcpTransport};
use super::{ConnectionDetails, Connector, LazyBuffers, Transport, TransportAdapter};

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

        let proxy_addrs = details
            .resolver
            .resolve(proxy.uri(), details.config, details.timeout)?;

        // Connect to the proxy like TcpConnector connects to a server, within the
        // connect timeout and trying each resolved address.
        let stream = tcp::try_connect(
            &proxy_addrs,
            details.now,
            details.timeout,
            details.current_time.clone(),
            details.config,
        )?;

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );

        // Like the CONNECT proxy handshake, the SOCKS handshake goes through the
        // transport, which gives every read and write the connect timeout.
        let mut w = TransportAdapter::new(
            TcpTransport::new(stream, buffers),
            details.current_time.clone(),
        );
        w.set_timeout(details.timeout);

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
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::config::Config;
    use crate::http::Uri;
    use crate::transport::{NextTimeout, TcpConnector};
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

    // SOCKS, then plain TCP, without the in-memory transport of the _test feature.
    fn agent(proxy: Proxy, timeout_global: Option<Duration>) -> Agent {
        let config = Agent::config_builder()
            .proxy(Some(proxy))
            .timeout_global(timeout_global)
            .build();
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

        let mut response = agent(proxy, None).get(&url).call().unwrap();
        assert_eq!(response.body_mut().read_to_string().unwrap(), "ok");
    }

    #[test]
    fn silent_proxy_times_out() {
        for protocol in ["socks4", "socks4a", "socks5", "socks5h"] {
            let silent = TcpListener::bind("127.0.0.1:0").unwrap();
            let proxy = format!("{}://{}", protocol, silent.local_addr().unwrap());
            let proxy = Proxy::new(&proxy).unwrap();

            // Accepts the connection, but never answers the handshake. Returns
            // whether ureq hung up before the proxy gave up after 5 seconds.
            let hung_up = thread::spawn(move || {
                let (mut stream, _) = silent.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream.read_to_end(&mut Vec::new()).is_ok()
            });

            let limit = Duration::from_millis(200);
            let start = Instant::now();
            let error = agent(proxy, Some(limit))
                .get("http://127.0.0.1:1/")
                .call()
                .unwrap_err();
            let elapsed = start.elapsed();

            assert!(
                matches!(error, Error::Timeout(Timeout::Global)),
                "{protocol}: {error:?}"
            );
            assert!(
                elapsed < limit + Duration::from_secs(1),
                "{protocol}: timed out after {elapsed:?}"
            );
            assert!(hung_up.join().unwrap(), "{protocol}: connection left open");
        }
    }

    #[test]
    fn request_through_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("socks5h://{}", listener.local_addr().unwrap());
        let proxy = Proxy::new(&proxy).unwrap();

        // Neither side waits forever if the handshake goes wrong.
        let limit = Duration::from_secs(5);

        // Answers the handshake, then the request as if it were the target.
        let proxy_side = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(limit)).unwrap();
            let mut greeting = [0; 3];
            stream.read_exact(&mut greeting).unwrap();
            stream.write_all(&[5, 0]).unwrap();
            let mut request = [0; 18];
            stream.read_exact(&mut request).unwrap();
            stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
            respond_ok(&mut stream);
            [&greeting[..], &request[..]].concat()
        });

        let mut response = agent(proxy, Some(limit))
            .get("http://example.com/")
            .call()
            .unwrap();
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
            self.replies.read(buf)
        }
    }

    impl Write for Script {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.sent.extend_from_slice(buf);
            Ok(buf.len())
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
            let (result, script) = script(proxy, *target, replies);
            result.unwrap_or_else(|e| panic!("{proxy}: {e}"));
            assert_eq!(script.sent, sent.concat(), "{proxy}");

            // Nothing is left unread to be mistaken for the HTTP response.
            let replies = script.replies.get_ref().len() as u64;
            assert_eq!(script.replies.position(), replies, "{proxy}");
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
}
