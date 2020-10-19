use crate::unit::Unit;
use crate::{error::Error, Agent};
use crate::{stream::Stream, AgentBuilder};
use once_cell::sync::Lazy;
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, net::ToSocketAddrs};

mod agent_test;
mod auth;
mod body_read;
mod body_send;
mod query_string;
mod range;
mod redirect;
mod simple;
pub(crate) mod testserver;
mod timeout;

// An agent to be installed by default for tests and doctests, such
// that all hostnames resolve to a TestServer on localhost.
pub(crate) fn test_agent() -> Agent {
    use std::io;
    use std::net::{SocketAddr, TcpStream};
    let testserver = testserver::TestServer::new(|mut stream: TcpStream| -> io::Result<()> {
        testserver::read_headers(&stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\n")?;
        stream.write_all(b"Transfer-Encoding: chunked\r\n")?;
        stream.write_all(b"Content-Type: text/html; charset=ISO-8859-1\r\n")?;
        stream.write_all(b"\r\n")?;
        stream.write_all(b"7\r\n")?;
        stream.write_all(b"success\r\n")?;
        stream.write_all(b"0\r\n")?;
        stream.write_all(b"\r\n")?;
        Ok(())
    });
    // Slightly tricky thing here: we want to make sure the TestServer lives
    // as long as the agent. This is accomplished by `move`ing it into the
    // closure, which becomes owned by the agent.
    AgentBuilder::new()
        .resolver(move |h: &str| -> io::Result<Vec<SocketAddr>> {
            // Don't override resolution for HTTPS requests yet, since we
            // don't have a setup for an HTTPS testserver. Also, skip localhost
            // resolutions since those may come from a unittest that set up
            // its own, specific testserver.
            if h.ends_with(":443") || h.starts_with("localhost:") {
                return Ok(h.to_socket_addrs()?.collect::<Vec<_>>());
            }
            let addr: SocketAddr = format!("127.0.0.1:{}", testserver.port).parse().unwrap();
            Ok(vec![addr])
        })
        .build()
}

type RequestHandler = dyn Fn(&Unit) -> Result<Stream, Error> + Send + 'static;

pub(crate) static TEST_HANDLERS: Lazy<Arc<Mutex<HashMap<String, Box<RequestHandler>>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

pub(crate) fn set_handler<H>(path: &str, handler: H)
where
    H: Fn(&Unit) -> Result<Stream, Error> + Send + 'static,
{
    let mut handlers = TEST_HANDLERS.lock().unwrap();
    handlers.insert(path.to_string(), Box::new(handler));
}

#[allow(clippy::write_with_newline)]
pub fn make_response(
    status: u16,
    status_text: &str,
    headers: Vec<&str>,
    mut body: Vec<u8>,
) -> Result<Stream, Error> {
    let mut buf: Vec<u8> = vec![];
    write!(&mut buf, "HTTP/1.1 {} {}\r\n", status, status_text).ok();
    for hstr in headers.iter() {
        write!(&mut buf, "{}\r\n", hstr).ok();
    }
    write!(&mut buf, "\r\n").ok();
    buf.append(&mut body);
    let cursor = Cursor::new(buf);
    let write: Vec<u8> = vec![];
    Ok(Stream::Test(Box::new(cursor), write))
}

pub(crate) fn resolve_handler(unit: &Unit) -> Result<Stream, Error> {
    let mut handlers = TEST_HANDLERS.lock().unwrap();
    let path = unit.url.path();
    let handler = handlers.remove(path).unwrap();
    handler(unit)
}
