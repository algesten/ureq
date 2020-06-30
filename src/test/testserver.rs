use std::io::{self, BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

pub struct TestServer {
    pub port: u16,
    pub done: Arc<AtomicBool>,
}

// Read a stream until reaching a blank line, in order to consume
// request headers.
pub fn read_headers(stream: &TcpStream) {
    for line in BufReader::new(stream).lines() {
        if line.unwrap() == "" {
            break;
        }
    }
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
