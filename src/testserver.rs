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
    let testserver = TestServer::new(|mut stream: TcpStream| -> io::Result<()> {
        read_headers(&stream);
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

pub struct TestServer {
    pub port: u16,
    pub done: Arc<AtomicBool>,
}

pub struct TestHeaders(Vec<String>);

#[allow(dead_code)]
impl TestHeaders {
    // Return the path for a request, e.g. /foo from "GET /foo HTTP/1.1"
    #[cfg(feature = "cookies")]
    pub fn path(&self) -> &str {
        if self.0.len() == 0 {
            ""
        } else {
            &self.0[0].split(" ").nth(1).unwrap()
        }
    }

    #[cfg(feature = "cookies")]
    pub fn headers(&self) -> &[String] {
        &self.0[1..]
    }
}

// Read a stream until reaching a blank line, in order to consume
// request headers.
pub fn read_headers(stream: &TcpStream) -> TestHeaders {
    let mut results = vec![];
    for line in BufReader::new(stream).lines() {
        match line {
            Err(e) => {
                eprintln!("testserver: in read_headers: {}", e);
                break;
            }
            Ok(line) if line == "" => break,
            Ok(line) => results.push(line),
        };
    }
    TestHeaders(results)
}

impl TestServer {
    pub fn new(handler: fn(TcpStream) -> io::Result<()>) -> Self {
        let listener = TcpListener::bind("localhost:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = done.clone();
        thread::spawn(move || {
            let mut conn_count = -1;
            for stream in listener.incoming() {
                conn_count += 1;
                // first connect is always the test for checking that server is ready.
                if conn_count == 0 {
                    break;
                }
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
        loop {
            if let Err(e) = TcpStream::connect(format!("localhost:{}", port)) {
                match e.kind() {
                    io::ErrorKind::ConnectionRefused => {
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    _ => panic!("testserver: pre-connect with error {}", e),
                }
            } else {
                break;
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
        TcpStream::connect(format!("localhost:{}", self.port)).unwrap();
    }
}
