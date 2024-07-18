use std::io::Write;
use std::io::{BufRead, BufReader};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::{fmt, io, thread};

use http::Uri;

use crate::transport::time::{Duration, NextTimeout};
use crate::Error;

use super::{Buffers, ConnectionDetails, Connector, LazyBuffers, Transport};

#[derive(Default)]
pub(crate) struct TestConnector;

impl Connector for TestConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        if chained.is_some() {
            // The chained connection overrides whatever we were to open here.
            trace!("Skip");
            return Ok(chained);
        }
        let config = details.config;

        let uri = details.uri.clone();

        let buffers = LazyBuffers::new(config.input_buffer_size, config.output_buffer_size);

        let (tx1, rx1) = mpsc::sync_channel(10);
        let (tx2, rx2) = mpsc::sync_channel(10);

        thread::spawn(|| test_run(uri, rx1, tx2));

        let transport = TestTransport {
            buffers,
            tx: tx1,
            rx: SyncReceiver(Mutex::new(rx2)),
            connected: true,
        };

        Ok(Some(Box::new(transport)))
    }
}

fn test_run(uri: Uri, rx: Receiver<Vec<u8>>, tx: mpsc::SyncSender<Vec<u8>>) {
    let mut reader = BufReader::new(RxRead(rx));
    let mut writer = TxWrite(tx);
    let uri = uri.to_string();

    println!("{}", uri);

    let mut lines: Vec<String> = Vec::new();
    loop {
        let mut s = String::new();
        match reader.read_line(&mut s) {
            Ok(_) => {
                if s.trim().is_empty() {
                    break;
                }
                lines.push(s.trim().to_string());
            }
            Err(_) => panic!("test request disconnected"),
        }
    }

    if uri.contains("www.google.com") {
        write!(
            &mut writer,
            "HTTP/1.1 200 OK\r\n\
            Content-Type: text/html;charset=ISO-8859-1\r\n\
            \r\n\
            ureq test server here"
        )
        .ok();
        return;
    } else if uri.contains("/bytes/100") {
        write!(
            &mut writer,
            "HTTP/1.1 200 OK\r\n\
            Content-Type: application/octet-stream\r\n\
            Content-Length: 100\r\n\
            \r\n"
        )
        .ok();
        write!(&mut writer, "{}", "1".repeat(100)).ok();
        return;
    } else if uri.contains("/get") {
        write!(
            &mut writer,
            "HTTP/1.1 200 OK\r\n\
            Content-Type: application/json\r\n\
            Content-Length: {}\r\n\
            \r\n",
            HTTPBIN_GET.as_bytes().len()
        )
        .ok();
        writer.write_all(HTTPBIN_GET.as_bytes()).ok();
        return;
    } else if uri.contains("/put") || uri.contains("/post") {
        write!(
            &mut writer,
            "HTTP/1.1 200 OK\r\n\
            Content-Type: application/json\r\n\
            Content-Length: {}\r\n\
            \r\n",
            HTTPBIN_PUT.as_bytes().len()
        )
        .ok();
        writer.write_all(HTTPBIN_PUT.as_bytes()).ok();
        return;
    } else if uri.contains("/robots.txt") {
        write!(
            &mut writer,
            "HTTP/1.1 200 OK\r\n\
            Content-Type: text/plain\r\n\
            Content-Length: 30\r\n\
            \r\n\
            User-agent: *\n\
            Disallow: /deny\n"
        )
        .ok();
        return;
    }

    panic!("test server unhandled url: {}", uri);
}

const HTTPBIN_GET: &str = r#"
{
  "args": {},
  "headers": {
    "Accept": "*/*",
    "Host": "httpbin.org",
    "User-Agent": "ureq/yeah",
    "X-Amzn-Trace-Id": "Root=1-6692ea70-181d2b331d51fb157521fba0"
  },
  "origin": "1.2.3.4",
  "url": "http://httpbin.org/get"
}"#;

const HTTPBIN_PUT: &str = r#"
{
  "args": {},
  "data": "foo",
  "files": {},
  "form": {},
  "headers": {
    "Accept": "*/*",
    "Content-Length": "3",
    "Content-Type": "application/octet-stream",
    "Host": "httpbin.org",
    "User-Agent": "curl/8.6.0",
    "X-Amzn-Trace-Id": "Root=1-6692eb75-0335ed3376385cc01144a4b6"
  },
  "json": null,
  "origin": "1.2.3.4",
  "url": "http://httpbin.org/put"
}"#;

struct RxRead(Receiver<Vec<u8>>);

impl io::Read for RxRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let v = match self.0.recv() {
            Ok(v) => v,
            Err(_) => return Ok(0), // remote side is gone
        };
        assert!(buf.len() > v.len());
        let max = buf.len().min(v.len());
        buf[..max].copy_from_slice(&v[..]);
        Ok(max)
    }
}

struct TxWrite(mpsc::SyncSender<Vec<u8>>);

impl io::Write for TxWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .send(buf.to_vec())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct TestTransport {
    buffers: LazyBuffers,
    tx: mpsc::SyncSender<Vec<u8>>,
    rx: SyncReceiver<Vec<u8>>,
    connected: bool,
}

impl Transport for TestTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, _timeout: NextTimeout) -> Result<(), Error> {
        let output = &self.buffers.output()[..amount];
        if self.tx.send(output.to_vec()).is_err() {
            self.connected = false;
        }
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<(), Error> {
        let input = self.buffers.input_mut();
        let buf = match self.rx.recv_timeout(timeout.after) {
            Ok(v) => v,
            Err(RecvTimeoutError::Timeout) => return Err(Error::Timeout(timeout.reason)),
            Err(RecvTimeoutError::Disconnected) => {
                self.connected = false;
                return Ok(());
            }
        };
        assert!(input.len() >= buf.len());
        let max = input.len().min(buf.len());
        input[..max].copy_from_slice(&buf[..]);
        self.buffers.add_filled(max);
        Ok(())
    }

    fn is_open(&mut self) -> bool {
        self.connected
    }

    fn is_tls(&self) -> bool {
        // Pretend this is tls to not get TLS wrappers
        true
    }
}

// Workaround for std::mpsc::Receiver not being Sync
struct SyncReceiver<T>(Mutex<Receiver<T>>);

impl<T> SyncReceiver<T> {
    fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        let lock = self.0.lock().unwrap();
        lock.recv_timeout(*timeout)
    }
}

impl fmt::Debug for TestConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestConnector").finish()
    }
}

impl fmt::Debug for TestTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestTransport").finish()
    }
}
