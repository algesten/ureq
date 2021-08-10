use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::{
    io::{self, BufRead, BufReader, Write},
    net::ToSocketAddrs,
};

use crate::{Agent, AgentBuilder};

// An agent to be installed by default for tests and doctests, such
// that all hostnames resolve to a TestServer on localhost.
pub(crate) fn test_agent() -> Agent {
    #[cfg(test)]
    let _ = env_logger::try_init();

    let testserver = TestServer::new(|mut stream: TcpStream| -> io::Result<()> {
        let headers = read_request(&stream);
        if headers.0.is_empty() {
            // no headers probably means it's the initial request to check test server is up.
        } else if headers.path() == "/status/200" {
            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n")?;
        } else if headers.path() == "/status/500" {
            stream.write_all(b"HTTP/1.1 500 Server Internal Error\r\n\r\n")?;
        } else if headers.path() == "/bytes/100" {
            stream.write_all(b"HTTP/1.1 200 OK\r\n")?;
            stream.write_all(b"Content-Length: 100\r\n")?;
            stream.write_all(b"\r\n")?;
            stream.write_all(&[0; 100])?;
        } else if headers.path() == "/hello_world.json" {
            stream.write_all(b"HTTP/1.1 200 OK\r\n")?;
            stream.write_all(b"\r\n")?;
            stream.write_all(br#"{"hello": "world"}"#)?;
        } else if headers.path() == "/status/301" {
            stream.write_all(b"HTTP/1.1 301 Found\r\n")?;
            stream.write_all(b"Location: /status/200\r\n")?;
            stream.write_all(b"\r\n")?;
        } else if headers.path() == "/status/307" {
            stream.write_all(b"HTTP/1.1 307 Found\r\n")?;
            stream.write_all(b"Location: /status/200\r\n")?;
            stream.write_all(b"\r\n")?;
        } else {
            stream.write_all(b"HTTP/1.1 200 OK\r\n")?;
            stream.write_all(b"Transfer-Encoding: chunked\r\n")?;
            stream.write_all(b"Content-Type: text/html; charset=ISO-8859-1\r\n")?;
            stream.write_all(b"\r\n")?;
            stream.write_all(b"7\r\n")?;
            stream.write_all(b"success\r\n")?;
            stream.write_all(b"0\r\n")?;
            stream.write_all(b"\r\n")?;
        }
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

pub struct TestServer {
    pub port: u16,
    pub done: Arc<AtomicBool>,
}

pub struct TestHeaders(Vec<String>);

#[allow(dead_code)]
impl TestHeaders {
    // Return the path for a request, e.g. /foo from "GET /foo HTTP/1.1"
    pub fn path(&self) -> &str {
        if self.0.is_empty() {
            ""
        } else {
            self.0[0].split(' ').nth(1).unwrap()
        }
    }

    #[cfg(feature = "cookies")]
    pub fn headers(&self) -> &[String] {
        &self.0[1..]
    }
}

// Read a stream until reaching a blank line, in order to consume
// request headers.
pub fn read_request(stream: &TcpStream) -> TestHeaders {
    let mut results = vec![];
    for line in BufReader::new(stream).lines() {
        match line {
            Err(e) => {
                eprintln!("testserver: in read_request: {}", e);
                break;
            }
            Ok(line) if line.is_empty() => break,
            Ok(line) => results.push(line),
        };
    }
    // Consume rest of body. TODO maybe capture the body for inspection in the test?
    // There's a risk stream is ended here, and fill_buf() would block.
    stream.set_nonblocking(true).ok();
    let mut reader = BufReader::new(stream);
    while let Ok(buf) = reader.fill_buf() {
        let amount = buf.len();
        if amount == 0 {
            break;
        }
        reader.consume(amount);
    }
    TestHeaders(results)
}

impl TestServer {
    pub fn new(handler: fn(TcpStream) -> io::Result<()>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = done.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                if let Err(e) = stream {
                    eprintln!("testserver: handling just-accepted stream: {}", e);
                    break;
                }
                if done.load(Ordering::SeqCst) {
                    break;
                } else {
                    thread::spawn(move || handler(stream.unwrap()));
                }
            }
        });
        // before returning from new(), ensure the server is ready to accept connections
        while let Err(e) = TcpStream::connect(format!("127.0.0.1:{}", port)) {
            match e.kind() {
                io::ErrorKind::ConnectionRefused => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                _ => eprintln!("testserver: pre-connect with error {}", e),
            }
        }
        TestServer {
            port,
            done: done_clone,
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        // Connect once to unblock the listen loop.
        if let Err(e) = TcpStream::connect(format!("localhost:{}", self.port)) {
            eprintln!("error dropping testserver: {}", e);
        }
    }
}
