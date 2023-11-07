use std::io;
use std::net::ToSocketAddrs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::{Agent, AgentBuilder};

#[cfg(not(feature = "testdeps"))]
fn test_server_handler(_stream: TcpStream) -> io::Result<()> {
    Ok(())
}

#[cfg(feature = "testdeps")]
fn test_server_handler(stream: TcpStream) -> io::Result<()> {
    use hootbin::serve_single;
    let o = stream.try_clone().expect("TcpStream to be clonable");
    let i = stream;
    match serve_single(i, o, "https://hootbin.test/") {
        Ok(()) => {}
        Err(e) => {
            if let hootbin::Error::Io(ioe) = &e {
                if ioe.kind() == io::ErrorKind::UnexpectedEof {
                    // accept this. the pre-connect below is always erroring.
                    return Ok(());
                }
            }

            println!("TestServer error: {:?}", e);
        }
    };
    Ok(())
}

// An agent to be installed by default for tests and doctests, such
// that all hostnames resolve to a TestServer on localhost.
pub(crate) fn test_agent() -> Agent {
    #[cfg(test)]
    let _ = env_logger::try_init();

    let testserver = TestServer::new(test_server_handler);
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

    pub(crate) fn to_headers(&self) -> Vec<crate::header::Header> {
        if self.0.len() <= 1 {
            return Vec::new();
        }

        let mut headers = Vec::new();
        for line in &self.0[1..] {
            let headerline = crate::header::HeaderLine::from(line.clone());
            let header = headerline.into_header().unwrap();
            headers.push(header);
        }
        headers
    }
}

// Read a stream until reaching a blank line, in order to consume
// request headers.
#[cfg(test)]
use std::io::{BufRead, BufReader};
#[cfg(test)]
pub fn read_request_headers(bufreader: &mut BufReader<&TcpStream>) -> TestHeaders {
    let mut results = vec![];
    loop {
        let mut line = String::new();
        bufreader.read_line(&mut line).unwrap();
        // Remove \r\n
        line.truncate(line.len().saturating_sub(2));

        if line.is_empty() {
            break;
        }
        results.push(line);
    }
    let mut body = Vec::new();
    body.append(&mut bufreader.buffer().to_vec());

    TestHeaders(results)
}

// Read whole body from a stream after reading the statusline and headers
#[cfg(test)]
pub fn read_request_body(
    bufreader: &mut BufReader<&TcpStream>,
    request_headers: &TestHeaders,
) -> Vec<u8> {
    let headers = request_headers.to_headers();

    // NOTE: Currently only requests with "Content-Length" is supported.
    let mut content_length = 0;
    for header in headers {
        if header.name() == "Content-Length" {
            content_length = header.value().unwrap().parse().unwrap();
        }
    }

    let mut bytes_read = 0;
    let mut body = Vec::new();

    // There's possibly already some data in the BufReader, read and consume
    // those first
    body.append(&mut bufreader.buffer().to_vec());
    bytes_read += body.len();
    bufreader.consume(body.len());

    while bytes_read < content_length {
        let buf = bufreader.fill_buf().unwrap();
        body.append(&mut buf.to_vec());
        let amount = buf.len();
        bytes_read += amount;
        bufreader.consume(amount);
    }

    body
}

// Read a stream as a request and return the headers
#[cfg(test)]
pub fn read_request(stream: &TcpStream) -> TestHeaders {
    let mut bufreader = BufReader::new(stream);

    let request_headers = read_request_headers(&mut bufreader);
    let _body = read_request_body(&mut bufreader, &request_headers);

    request_headers
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
