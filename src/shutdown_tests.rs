//! Tests for [`Agent::shutdown()`] against real sockets.
//!
//! The server never answers (or answers only in part), so a request blocks on
//! the socket until the shutdown interrupts it.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

use crate::config::Config;
use crate::http::Uri;
use crate::test::init_test_log;
use crate::transport::{NextTimeout, TcpConnector};
use crate::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use crate::{Agent, Error};

/// An agent with real TCP sockets and no timeouts. Without a shutdown, a
/// request against a silent server blocks forever.
fn agent() -> Agent {
    init_test_log();
    let config = Agent::config_builder().proxy(None).build();
    Agent::with_parts(config, TcpConnector::default(), LoopbackResolver)
}

/// Resolves the test server's `127.0.0.1:port` literally. The default resolver
/// answers with a fake address under the `_test` feature.
#[derive(Debug)]
struct LoopbackResolver;

impl Resolver for LoopbackResolver {
    fn resolve(&self, uri: &Uri, _: &Config, _: NextTimeout) -> Result<ResolvedSocketAddrs, Error> {
        let addr: SocketAddr = uri.authority().unwrap().as_str().parse().unwrap();
        let mut addrs = self.empty();
        addrs.push(addr);
        Ok(addrs)
    }
}

/// A server that answers each connection with `response` and then holds the
/// connection open. The accepted connections are handed to the test, which
/// keeps them open and can observe them being closed.
fn serve(response: &'static [u8]) -> (String, mpsc::Receiver<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            stream.write_all(response).unwrap();
            if tx.send(stream).is_err() {
                break;
            }
        }
    });
    (url, rx)
}

/// Run `f` on another thread. The result arrives on the receiver.
fn spawn<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> mpsc::Receiver<T> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

fn is_io(result: &Result<impl std::fmt::Debug, Error>) -> bool {
    matches!(result, Err(Error::Io(_)))
}

#[test]
fn shutdown_interrupts_request_awaiting_response() {
    let (url, accepted) = serve(b"");
    let agent = agent();

    let worker = {
        let agent = agent.clone();
        spawn(move || agent.get(&url).call().map(|_| ()))
    };

    // The server never answers, so the worker blocks reading the response.
    let _conn = accepted.recv().expect("request to connect");

    agent.shutdown();

    let result = worker.recv().expect("shutdown to interrupt the request");
    assert!(is_io(&result), "{result:?}");
}

#[test]
fn shutdown_interrupts_body_read() {
    // The body is 10 bytes, but only 5 arrive.
    let (url, accepted) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhello");
    let agent = agent();

    let (responded_tx, responded) = mpsc::channel();
    let worker = {
        let agent = agent.clone();
        spawn(move || {
            let mut res = agent.get(&url).call()?;
            responded_tx.send(()).unwrap();
            res.body_mut().read_to_string()
        })
    };

    let _conn = accepted.recv().expect("request to connect");
    responded.recv().expect("response head to arrive");

    // The worker is now blocked reading the rest of the body.
    agent.shutdown();

    let result = worker.recv().expect("shutdown to interrupt the body read");
    assert!(is_io(&result), "{result:?}");
}

#[test]
fn shutdown_closes_idle_pooled_connection() {
    let (url, accepted) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let agent = agent();

    agent.get(&url).call().unwrap();
    let mut conn = accepted.recv().unwrap();
    assert_eq!(agent.pool_count(), 1);

    // Clones share the pool, so shutting down any of them is enough.
    agent.clone().shutdown();

    assert_eq!(agent.pool_count(), 0);

    // The server sees EOF after the request bytes it has not read yet.
    conn.read_to_end(&mut Vec::new()).unwrap();
}

#[test]
fn shutdown_refuses_new_requests() {
    let (url, accepted) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let agent = agent();
    let clone = agent.clone();

    agent.shutdown();

    let err = clone.get(&url).call().unwrap_err();
    assert!(
        matches!(&err, Error::Io(e) if e.kind() == io::ErrorKind::NotConnected),
        "{err:?}"
    );

    // Refused before connecting.
    assert!(accepted.try_recv().is_err());
}

#[test]
#[cfg(feature = "socks-proxy")]
fn shutdown_interrupts_socks_handshake() {
    use crate::Proxy;
    use crate::transport::SocksConnector;

    // The proxy accepts the connection but never answers the handshake.
    let (proxy_url, accepted) = serve(b"");
    let proxy_addr = proxy_url
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let proxy = Proxy::new(&format!("socks5://{proxy_addr}")).unwrap();

    init_test_log();
    let config = Agent::config_builder().proxy(Some(proxy)).build();
    let agent = Agent::with_parts(config, SocksConnector::default(), LoopbackResolver);

    let worker = {
        let agent = agent.clone();
        spawn(move || agent.get("http://127.0.0.1:1/").call().map(|_| ()))
    };

    // The connector registers the proxy socket before the handshake, which now
    // waits for the proxy to answer.
    let _conn = accepted.recv().expect("proxy to be connected");

    agent.shutdown();

    let result = worker.recv().expect("shutdown to interrupt the handshake");
    assert!(result.is_err(), "{result:?}");
}
