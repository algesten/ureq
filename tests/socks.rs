#![cfg(feature = "socks-proxy")]

// CONNECT equivalents of socks 0.3.4's src/v4.rs and src/v5.rs tests:
// https://github.com/sfackler/rust-socks/tree/4e7463d7e8c6a17485a86b44853ab6bc8d65c57a
// Its tests run against Dante and Google. Here a scripted loopback proxy checks
// the handshake and relays HTTP to a local origin, without DNS or a daemon.
// BIND and UDP ASSOCIATE tests exercise APIs ureq never used.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{Connector, NextTimeout, SocksConnector, TcpConnector};
use ureq::{Agent, Error, Proxy, ProxyProtocol};

const GRANTED4: &[u8] = &[0, 90, 0, 0, 0, 0, 0, 0];
const GRANTED5: &[u8] = &[5, 0, 0, 1, 0, 0, 0, 0, 0, 0];

// The bytes a client must send, followed by the proxy's reply.
type Exchange = (Vec<u8>, Vec<u8>);

#[derive(Debug)]
struct LocalResolver(Arc<Mutex<Vec<String>>>);

impl Resolver for LocalResolver {
    fn resolve(&self, uri: &Uri, _: &Config, _: NextTimeout) -> Result<ResolvedSocketAddrs, Error> {
        let host = uri.host().unwrap();
        self.0.lock().unwrap().push(host.to_string());
        let ip = if host == "origin.test" {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            host.parse().map_err(|_| Error::HostNotFound)?
        };
        let mut addrs = self.empty();
        addrs.push(SocketAddr::new(ip, uri.port_u16().unwrap()));
        Ok(addrs)
    }
}

fn agent(proxy: Proxy) -> (Agent, Arc<Mutex<Vec<String>>>) {
    let lookups = Arc::new(Mutex::new(Vec::new()));
    let config = Agent::config_builder().proxy(Some(proxy)).build();
    // Exercise real sockets even when CI enables the _test transport.
    let connector = ().chain(SocksConnector::default()).chain(TcpConnector::default());
    let agent = Agent::with_parts(config, connector, LocalResolver(Arc::clone(&lookups)));
    (agent, lookups)
}

fn accept(listener: TcpListener) -> TcpStream {
    let (stream, _) = listener.accept().unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn read_headers(stream: &mut TcpStream) -> Vec<u8> {
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        headers.push(byte[0]);
        assert!(headers.len() <= 16 * 1024, "request headers too large");
    }
    headers
}

fn origin() -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut stream = accept(listener);
        let headers = read_headers(&mut stream);
        assert!(headers.starts_with(b"GET /through-socks HTTP/1.1\r\n"));
        let response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Length: 16\r\nConnection: close\r\n\r\n",
            "hello over socks",
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (address, server)
}

fn proxy_server(
    exchanges: Vec<Exchange>,
    origin: Option<SocketAddr>,
) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut stream = accept(listener);
        for (expected, reply) in exchanges {
            let mut actual = vec![0; expected.len()];
            stream.read_exact(&mut actual).unwrap();
            assert_eq!(actual, expected, "unexpected SOCKS handshake bytes");
            stream.write_all(&reply).unwrap();
        }
        if let Some(origin) = origin {
            let mut upstream = TcpStream::connect(origin).unwrap();
            upstream.write_all(&read_headers(&mut stream)).unwrap();
            io::copy(&mut upstream, &mut stream).unwrap();
        } else {
            // Authentication failed or the client rejected its credentials.
            assert_eq!(stream.read(&mut [0]).unwrap(), 0, "client must close");
        }
    });
    (address, server)
}

fn configured_proxy(
    address: SocketAddr,
    protocol: ProxyProtocol,
    resolve_target: bool,
    credentials: Option<(&str, &str)>,
) -> Proxy {
    let mut builder = Proxy::builder(protocol)
        .host("127.0.0.1")
        .port(address.port())
        .resolve_target(resolve_target);
    if let Some((user, pass)) = credentials {
        builder = builder.username(user).password(pass);
    }
    builder.build().unwrap()
}

fn http_exchange(
    protocol: ProxyProtocol,
    resolve_target: bool,
    credentials: Option<(&str, &str)>,
    exchanges: impl FnOnce(u16) -> Vec<Exchange>,
) {
    let (target, origin) = origin();
    let (address, proxy) = proxy_server(exchanges(target.port()), Some(target));
    let config = configured_proxy(address, protocol, resolve_target, credentials);
    let (agent, lookups) = agent(config);
    let mut response = agent
        .get(format!(
            "http://origin.test:{}/through-socks",
            target.port()
        ))
        .call()
        .unwrap();
    proxy.join().unwrap();
    origin.join().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.body_mut().read_to_string().unwrap(),
        "hello over socks"
    );
    let lookups = lookups.lock().unwrap();
    assert!(lookups.iter().any(|host| host == "127.0.0.1"));
    assert_eq!(
        lookups.iter().any(|host| host == "origin.test"),
        resolve_target,
        "target DNS must happen on the configured side of the proxy"
    );
}

fn ipv4_connect(port: u16) -> Vec<u8> {
    vec![5, 1, 0, 1, 127, 0, 0, 1, (port >> 8) as u8, port as u8]
}

fn no_auth(request: Vec<u8>) -> Vec<Exchange> {
    vec![(vec![5, 1, 0], vec![5, 0]), (request, GRANTED5.to_vec())]
}

fn password_request(user: &str, pass: &str) -> Vec<u8> {
    [
        vec![1, user.len() as u8],
        user.as_bytes().to_vec(),
        vec![pass.len() as u8],
        pass.as_bytes().to_vec(),
    ]
    .concat()
}

// socks::v4::test::google
#[test]
fn socks4_connect() {
    http_exchange(ProxyProtocol::Socks4, true, None, |port| {
        vec![(
            vec![4, 1, (port >> 8) as u8, port as u8, 127, 0, 0, 1, 0],
            GRANTED4.to_vec(),
        )]
    });
}

// socks::v4::test::google_dns (ignored upstream; exercised here)
#[test]
fn socks4a_connect_with_proxy_dns() {
    http_exchange(ProxyProtocol::Socks4A, false, None, |port| {
        vec![(
            [
                vec![4, 1, (port >> 8) as u8, port as u8, 0, 0, 0, 1, 0],
                b"origin.test\0".to_vec(),
            ]
            .concat(),
            GRANTED4.to_vec(),
        )]
    });
}

// socks::v5::test::google_no_auth
#[test]
fn socks5_connect_no_auth() {
    http_exchange(ProxyProtocol::Socks5, true, None, |port| {
        no_auth(ipv4_connect(port))
    });
}

// socks::v5::test::google_with_password
#[test]
fn socks5_connect_with_password() {
    let credentials = Some(("testuser", "testpass"));
    http_exchange(ProxyProtocol::Socks5, true, credentials, |port| {
        vec![
            (vec![5, 2, 2, 0], vec![5, 2]),
            (password_request("testuser", "testpass"), vec![1, 0]),
            (ipv4_connect(port), GRANTED5.to_vec()),
        ]
    });
}

// socks::v5::test::google_dns
#[test]
fn socks5_connect_with_proxy_dns() {
    http_exchange(ProxyProtocol::Socks5h, false, None, |port| {
        no_auth(
            [
                vec![5, 1, 0, 3, 11],
                b"origin.test".to_vec(),
                port.to_be_bytes().to_vec(),
            ]
            .concat(),
        )
    });
}

// The authentication path in upstream's bind_with_password_supported_but_no_auth_used
// is shared with CONNECT. Credentials must remain unused when the proxy chooses 0.
#[test]
fn socks5_connect_without_using_offered_credentials() {
    let credentials = Some(("unused_and_invalid_username", ""));
    http_exchange(ProxyProtocol::Socks5, true, credentials, |port| {
        vec![
            (vec![5, 2, 2, 0], vec![5, 0]),
            (ipv4_connect(port), GRANTED5.to_vec()),
        ]
    });
}

fn authentication_error(
    credentials: Option<(&str, &str)>,
    exchanges: Vec<Exchange>,
    kind: io::ErrorKind,
    reason: &str,
) {
    let (address, proxy) = proxy_server(exchanges, None);
    let config = configured_proxy(address, ProxyProtocol::Socks5, true, credentials);
    let (agent, _) = agent(config);
    let result = agent.get("http://127.0.0.1:1/").call();
    proxy.join().unwrap();
    let error = result.unwrap_err();
    let Error::Io(error) = error else {
        panic!("expected an I/O error, got {error:?}");
    };
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), reason);
}

// socks::v5::test::incorrect_password
#[test]
fn socks5_incorrect_password() {
    authentication_error(
        Some(("testuser", "invalid")),
        vec![
            (vec![5, 2, 2, 0], vec![5, 2]),
            (password_request("testuser", "invalid"), vec![1, 1]),
        ],
        io::ErrorKind::PermissionDenied,
        "password authentication failed",
    );
}

// socks::v5::test::auth_method_not_supported
#[test]
fn socks5_auth_method_not_supported() {
    authentication_error(
        None,
        vec![(vec![5, 1, 0], vec![5, 0xff])],
        io::ErrorKind::Other,
        "no acceptable auth methods",
    );
}

// socks::v5::test::username_and_password_length, including its six boundary cases.
#[test]
fn socks5_username_and_password_length() {
    for (user_len, pass_len, invalid) in [
        (1, 1, None),
        (255, 255, None),
        (0, 255, Some("invalid username")),
        (256, 255, Some("invalid username")),
        (255, 0, Some("invalid password")),
        (255, 256, Some("invalid password")),
    ] {
        let user = "x".repeat(user_len);
        let pass = "x".repeat(pass_len);
        let mut exchanges = vec![(vec![5, 2, 2, 0], vec![5, 2])];
        let (kind, reason) = if let Some(reason) = invalid {
            (io::ErrorKind::InvalidInput, reason)
        } else {
            // Valid lengths reach authentication; the proxy rejects these credentials.
            exchanges.push((password_request(&user, &pass), vec![1, 1]));
            (
                io::ErrorKind::PermissionDenied,
                "password authentication failed",
            )
        };
        authentication_error(Some((&user, &pass)), exchanges, kind, reason);
    }
}
