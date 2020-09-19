use std::io::{self, BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

pub struct TestServer {
    pub port: u16,
    pub done: Arc<AtomicBool>,
}

pub struct TestHeaders(Vec<String>);

impl TestHeaders {
    // Return the path for a request, e.g. /foo from "GET /foo HTTP/1.1"
    #[cfg(feature = "cookie")]
    pub fn path(&self) -> &str {
        if self.0.len() == 0 {
            ""
        } else {
            &self.0[0].split(" ").nth(1).unwrap()
        }
    }

    #[cfg(feature = "cookie")]
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
            Err(e) => panic!(e),
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
            for stream in listener.incoming() {
                thread::spawn(move || handler(stream.unwrap()));
                if done.load(Ordering::Relaxed) {
                    break;
                }
            }
            println!("testserver on {} exiting", port);
        });
        TestServer {
            port,
            done: done_clone,
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        // Connect once to unblock the listen loop.
        TcpStream::connect(format!("localhost:{}", self.port)).unwrap();
    }
}
