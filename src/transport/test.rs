#![allow(clippy::type_complexity)]

use std::cell::RefCell;
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::{fmt, io, thread};

use http::{Method, Request, Uri};

use crate::transport::time::Duration;
use crate::Error;

use super::{Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport};

#[derive(Default)]
pub(crate) struct TestConnector;

thread_local!(static HANDLERS: RefCell<Vec<TestHandler>> = const { RefCell::new(Vec::new()) });

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

        let mut handlers = HANDLERS.with(|h| (*h).borrow().clone());
        setup_default_handlers(&mut handlers);

        thread::spawn(|| test_run(uri, rx1, tx2, handlers));

        let transport = TestTransport {
            buffers,
            tx: tx1,
            rx: SyncReceiver(Mutex::new(rx2)),
            connected: true,
        };

        Ok(Some(Box::new(transport)))
    }
}

impl TestHandler {
    fn new(
        pattern: &'static str,
        handler: impl Fn(Uri, Request<()>, &mut dyn Write) -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        TestHandler {
            pattern,
            handler: Arc::new(handler),
        }
    }
}

/// Helper for **_test** feature tests.
pub fn set_handler(pattern: &'static str, status: u16, headers: &[(&str, &str)], body: &[u8]) {
    // Convert headers to a big string
    let mut headers_s = String::new();
    for (k, v) in headers {
        headers_s.push_str(&format!("{}: {}\r\n", k, v));
    }

    // Convert body to an owned vec
    let body = body.to_vec();

    let handler = TestHandler::new(pattern, move |_uri, _req, w| {
        write!(
            w,
            "HTTP/1.1 {} OK\r\n\
            {}\
            \r\n",
            status, headers_s
        )?;
        w.write_all(&body)
    });

    HANDLERS.with(|h| (*h).borrow_mut().push(handler));
}

#[derive(Clone)]
struct TestHandler {
    pattern: &'static str,
    handler: Arc<dyn Fn(Uri, Request<()>, &mut dyn Write) -> io::Result<()> + Sync + Send>,
}

fn test_run(
    uri: Uri,
    rx: Receiver<Vec<u8>>,
    tx: mpsc::SyncSender<Vec<u8>>,
    handlers: Vec<TestHandler>,
) {
    let mut reader = BufReader::new(RxRead(rx));
    let mut writer = TxWrite(tx);
    let uri_s = uri.to_string();

    let req = loop {
        let input = reader.fill_buf().expect("test fill_buf");
        let maybe =
            ureq_proto::parser::try_parse_request::<100>(input).expect("test parse request");
        if let Some((amount, req)) = maybe {
            reader.consume(amount);
            break req;
        } else {
            continue;
        }
    };

    for handler in handlers {
        if uri_s.contains(handler.pattern) {
            (handler.handler)(uri, req, &mut writer).expect("test handler to not fail");
            return;
        }
    }

    panic!("test server unhandled url: {}", uri);
}

fn setup_default_handlers(handlers: &mut Vec<TestHandler>) {
    fn maybe_add(handler: TestHandler, handlers: &mut Vec<TestHandler>) {
        let already_declared = handlers.iter().any(|h| h.pattern == handler.pattern);
        if !already_declared {
            handlers.push(handler);
        }
    }

    maybe_add(
        TestHandler::new("www.google.com", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: text/html;charset=ISO-8859-1\r\n\
                set-cookie: AEC=AVYB7cpadYFS8ZgaioQ17NnxHl1QcSQ_2aH2WEIg1KGDXD5kjk2HhpGVhfk; \
                    expires=Mon, 14-Apr-2025 17:23:39 GMT; path=/; domain=.google.com; \
                    Secure; HttpOnly; SameSite=lax\r\n\
                set-cookie: __Secure-ENID=23.SE=WaDe-mOBoV2nk-IwHr73boNt6dYcjzQh1X_k8zv2UmUXBL\
                    m80a3pzLJyx1N1NOqBxDDOR8OJyvuNYw5phFf0VnbqzVtcKPijo2FY8O_vymzyc7x2VwFhGlgU\
                    WXSWYinjWL7Zvz_EOcA4kfnEXweW5ZDzLrvaLuBIrz5CA_-454AMIXpDiZAVPChCawbkzMptAr\
                    lMTikkon2EQVXsicqq1XnrMEMPZR5Ld2JC6lpBM8A; expires=Sun, 16-Nov-2025 09:41:57 \
                    GMT; path=/; domain=.google.com; Secure; HttpOnly; SameSite=lax\r\n\
                \r\n\
                ureq test server here"
            )
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("example.com", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: text/html;charset=UTF-8\r\n\
                \r\n\
                ureq test server here"
            )
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/bytes/100", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/octet-stream\r\n\
                Content-Length: 100\r\n\
                \r\n"
            )?;
            write!(w, "{}", "1".repeat(100))
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/get", |_uri, req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json\r\n\
                Content-Length: {}\r\n\
                \r\n",
                HTTPBIN_GET.as_bytes().len()
            )?;
            if req.method() != Method::HEAD {
                w.write_all(HTTPBIN_GET.as_bytes())?;
            }
            Ok(())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/head", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json\r\n\
                Content-Length: {}\r\n\
                \r\n",
                HTTPBIN_GET.as_bytes().len()
            )
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/put", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json\r\n\
                Content-Length: {}\r\n\
                \r\n",
                HTTPBIN_PUT.as_bytes().len()
            )?;
            w.write_all(HTTPBIN_PUT.as_bytes())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/post", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                    Content-Type: application/json\r\n\
                    Content-Length: {}\r\n\
                    \r\n",
                HTTPBIN_PUT.as_bytes().len()
            )?;
            w.write_all(HTTPBIN_PUT.as_bytes())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/delete", |_uri, _req, w| {
            write!(w, "HTTP/1.1 200 OK\r\n\r\ndeleted\n")
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/robots.txt", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: text/plain\r\n\
                Content-Length: 30\r\n\
                \r\n\
                User-agent: *\n\
                Disallow: /deny\n"
            )
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/json", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json\r\n\
                Content-Length: {}\r\n\
                \r\n",
                HTTPBIN_JSON.as_bytes().len()
            )?;
            w.write_all(HTTPBIN_JSON.as_bytes())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/redirect-to", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 302 OK\r\n\
                Location: /get\r\n\
                Content-Length: 22\r\n\
                \r\n\
                You've been redirected\
                ",
            )
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/cookie-test", |_uri, req, w| {
            let mut all: Vec<_> = req
                .headers()
                .get_all("cookie")
                .iter()
                .map(|c| c.to_str().unwrap())
                .collect();

            all.sort();

            assert_eq!(all, ["a=1;b=2"]);

            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                content-length: 2\r\n\
                \r\n\
                ok",
            )
        }),
        handlers,
    );

    #[cfg(feature = "charset")]
    {
        let (cow, _, _) =
            encoding_rs::WINDOWS_1252.encode("HTTP/1.1 302 Déplacé Temporairement\r\n\r\n");
        let bytes = cow.to_vec();

        maybe_add(
            TestHandler::new("/non-ascii-reason", move |_uri, _req, w| {
                w.write_all(&bytes)?;
                Ok(())
            }),
            handlers,
        );
    }
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

const HTTPBIN_JSON: &str = r#"
{
  "slideshow": {
    "author": "Yours Truly",
    "date": "date of publication",
    "slides": [
      {
        "title": "Wake up to WonderWidgets!",
        "type": "all"
      },
      {
        "items": [
          "Why <em>WonderWidgets</em> are great",
          "Who <em>buys</em> WonderWidgets"
        ],
        "title": "Overview",
        "type": "all"
      }
    ],
    "title": "Sample Slide Show"
  }
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

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        let input = self.buffers.input_append_buf();
        let buf = match self.rx.recv_timeout(timeout.after) {
            Ok(v) => v,
            Err(RecvTimeoutError::Timeout) => return Err(Error::Timeout(timeout.reason)),
            Err(RecvTimeoutError::Disconnected) => {
                trace!("Test server disconnected");
                self.connected = false;
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "test server disconnected",
                )));
            }
        };
        assert!(input.len() >= buf.len());
        let max = input.len().min(buf.len());
        input[..max].copy_from_slice(&buf[..]);
        self.buffers.input_appended(max);
        Ok(max > 0)
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
