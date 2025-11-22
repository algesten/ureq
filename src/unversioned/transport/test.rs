#![allow(clippy::type_complexity)]

use std::cell::RefCell;
use std::io::Write;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::{fmt, io, thread};

use http::{Method, Request, Uri};
use ureq_proto::parser::try_parse_request;

use crate::http;
use crate::Error;

use super::chain::Either;
use super::time::Duration;
use super::{Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport};

#[derive(Default)]
pub(crate) struct TestConnector;

thread_local!(static HANDLERS: RefCell<Vec<TestHandler>> = const { RefCell::new(Vec::new()) });

impl<In: Transport> Connector<In> for TestConnector {
    type Out = Either<In, TestTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        if chained.is_some() {
            // The chained connection overrides whatever we were to open here.
            trace!("Skip");
            return Ok(chained.map(Either::A));
        }
        let config = details.config;

        let uri = details.uri.clone();
        debug!("Test uri: {}", uri);

        let buffers = LazyBuffers::new(config.input_buffer_size(), config.output_buffer_size());

        let (tx1, rx1) = mpsc::sync_channel(10);
        let (tx2, rx2) = mpsc::sync_channel(10);

        let mut handlers = HANDLERS.with(|h| (*h).borrow().clone());
        setup_default_handlers(&mut handlers);

        thread::spawn(|| test_run(uri, rx1, tx2, handlers));

        let transport = TestTransport {
            buffers,
            tx: tx1,
            rx: SyncReceiver(Mutex::new(rx2)),
            connected_tx: true,
            connected_rx: true,
        };

        Ok(Some(Either::B(transport)))
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
#[cfg(feature = "_test")]
#[doc(hidden)]
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

/// Helper for **_test** feature tests that need to inspect the request.
#[cfg(feature = "_test")]
#[doc(hidden)]
pub fn set_handler_cb(
    pattern: &'static str,
    status: u16,
    headers: &[(&str, &str)],
    body: &[u8],
    cb: impl Fn(&Request<()>) + Send + Sync + 'static,
) {
    // Convert headers to a big string
    let mut headers_s = String::new();
    for (k, v) in headers {
        headers_s.push_str(&format!("{}: {}\r\n", k, v));
    }

    // Convert body to an owned vec
    let body = body.to_vec();

    let handler = TestHandler::new(pattern, move |_uri, req, w| {
        // Run the request check (can panic if assertions fail)
        cb(&req);

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
    let mut reader = SaneBufReader(Some(RxRead(rx)), vec![]);
    let mut writer = TxWrite(tx);
    let uri_s = uri.to_string();

    let req = loop {
        let input = reader.fill_buf().expect("test fill_buf");
        let maybe = try_parse_request::<100>(input).expect("test parse request");
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
                    expires=Mon, 14-Apr-2050 17:23:39 GMT; path=/; domain=.google.com; \
                    Secure; HttpOnly; SameSite=lax\r\n\
                set-cookie: __Secure-ENID=23.SE=WaDe-mOBoV2nk-IwHr73boNt6dYcjzQh1X_k8zv2UmUXBL\
                    m80a3pzLJyx1N1NOqBxDDOR8OJyvuNYw5phFf0VnbqzVtcKPijo2FY8O_vymzyc7x2VwFhGlgU\
                    WXSWYinjWL7Zvz_EOcA4kfnEXweW5ZDzLrvaLuBIrz5CA_-454AMIXpDiZAVPChCawbkzMptAr\
                    lMTikkon2EQVXsicqq1XnrMEMPZR5Ld2JC6lpBM8A; expires=Sun, 16-Nov-2050 09:41:57 \
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
        TestHandler::new("/bytes/200000000", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/octet-stream\r\n\
                Content-Length: 100\r\n\
                \r\n"
            )?;
            // We don't actually want 200MB of data in memory.
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
                HTTPBIN_GET.len()
            )?;
            if req.method() != Method::HEAD {
                w.write_all(HTTPBIN_GET.as_bytes())?;
            }
            Ok(())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/?query=foo", |_uri, req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json\r\n\
                Content-Length: {}\r\n\
                \r\n",
                HTTPBIN_GET.len()
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
                HTTPBIN_GET.len()
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
                HTTPBIN_PUT.len()
            )?;
            w.write_all(HTTPBIN_PUT.as_bytes())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/post", |_uri, req, w| {
            // Check if there's an x-verify-content-type header to verify against
            if let Some(expected_ct) = req.headers().get("x-verify-content-type") {
                let expected_ct_str = expected_ct.to_str().unwrap();
                let actual_ct = req.headers().get("content-type");

                match actual_ct {
                    Some(ct) => {
                        let actual_ct_str = ct.to_str().unwrap();
                        if expected_ct_str.starts_with("multipart/form-data") {
                            // For multipart, just check it starts with the expected prefix
                            assert!(
                                actual_ct_str.starts_with("multipart/form-data; boundary="),
                                "Expected multipart/form-data with boundary, got: {}",
                                actual_ct_str
                            );
                        } else {
                            assert_eq!(
                                actual_ct_str, expected_ct_str,
                                "Content-Type mismatch: expected '{}', got '{}'",
                                expected_ct_str, actual_ct_str
                            );
                        }
                    }
                    None => panic!(
                        "Expected Content-Type '{}' but no Content-Type header found",
                        expected_ct_str
                    ),
                }
            }

            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                    Content-Type: application/json\r\n\
                    Content-Length: {}\r\n\
                    \r\n",
                HTTPBIN_PUT.len()
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
                HTTPBIN_JSON.len()
            )?;
            w.write_all(HTTPBIN_JSON.as_bytes())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/redirect-to", |uri, _req, w| {
            let location = uri.query().unwrap();
            assert!(location.starts_with("url="));
            let location = &location[4..];
            let location = percent_encoding::percent_decode_str(location)
                .decode_utf8()
                .unwrap();
            write!(
                w,
                "HTTP/1.1 302 FOUND\r\n\
                Location: {}\r\n\
                Content-Length: 22\r\n\
                Connection: close\r\n\
                \r\n\
                You've been redirected\
                ",
                location
            )
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/partial-redirect", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 302 OK\r\n\
                Location: /get\r\n\
                set-cookie: AEC=AVYB7cpadYFS8ZgaioQ17NnxHl1QcSQ_2aH2WEIg1KGDXD5kjk2HhpGVhfk; \
                    expires=Mon, 14-Apr-2050 17:23:39 GMT; path=/; domain=.google.com; \
                    Secure; HttpOnly; SameSite=lax\r\n\
                " // deliberately omit final \r\n
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

    maybe_add(
        TestHandler::new("/connect-proxy", |_uri, req, w| {
            assert_eq!(req.uri(), "httpbin.org:80");
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                \r\n\
                HTTP/1.1 200 OK\r\n\
                Content-Type: application/json\r\n\
                Content-Length: {}\r\n\
                \r\n",
                HTTPBIN_GET.len()
            )?;
            w.write_all(HTTPBIN_GET.as_bytes())?;
            Ok(())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/fnord", |_uri, req, w| {
            assert_eq!(req.method().as_str(), "FNORD");

            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json\r\n\
                Content-Length: {}\r\n\
                \r\n",
                HTTPBIN_GET.len()
            )?;
            if req.method() != Method::HEAD {
                w.write_all(HTTPBIN_GET.as_bytes())?;
            }
            Ok(())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/1chunk-abort", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Transfer-Encoding: chunked\r\n\
                \r\n\
                2\r\n\
                OK\r\n\
                0\r\n<hangup>",
            )?;
            Ok(())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/2chunk-abort", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Transfer-Encoding: chunked\r\n\
                \r\n\
                2\r\n\
                OK\r\n\
                0\r\n\
                \r<hangup>", // missing \n
            )?;
            Ok(())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/3chunk-abort", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Transfer-Encoding: chunked\r\n\
                \r\n\
                2\r\n\
                OK\r\n\
                0\r\n\
                \r\n<hangup>",
            )?;
            Ok(())
        }),
        handlers,
    );

    maybe_add(
        TestHandler::new("/4chunk-abort", |_uri, _req, w| {
            write!(
                w,
                "HTTP/1.1 200 OK\r\n\
                Transfer-Encoding: chunked\r\n\
                \r\n\
                2\r\n\
                OK\r\n\
                0\r\n\
                \r\n",
            )?;
            Ok(())
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
        assert!(buf.len() >= v.len(), "{} > {}", buf.len(), v.len());
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

struct SaneBufReader<R: io::Read>(Option<R>, Vec<u8>);

impl<R: io::Read> io::Read for SaneBufReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.1.is_empty() {
            let max = buf.len().min(self.1.len());
            buf[..max].copy_from_slice(&self.1[..max]);
            self.1.drain(..max);
            return Ok(max);
        }

        let Some(reader) = &mut self.0 else {
            return Ok(0);
        };
        reader.read(buf)
    }
}

impl<R: io::Read> SaneBufReader<R> {
    pub fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let Some(reader) = &mut self.0 else {
            return Ok(&self.1);
        };

        let l = self.1.len();
        self.1.resize(l + 1024, 0);
        let buf = &mut self.1[l..];
        let n = reader.read(buf)?;
        if n == 0 {
            self.0 = None;
        }
        self.1.truncate(l + n);
        Ok(&self.1)
    }

    pub fn consume(&mut self, n: usize) {
        self.1.drain(..n);
    }
}

pub(crate) struct TestTransport {
    buffers: LazyBuffers,
    tx: mpsc::SyncSender<Vec<u8>>,
    rx: SyncReceiver<Vec<u8>>,
    connected_tx: bool,
    connected_rx: bool,
}

impl Transport for TestTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, _timeout: NextTimeout) -> Result<(), Error> {
        let output = &self.buffers.output()[..amount];
        if self.tx.send(output.to_vec()).is_err() {
            self.connected_tx = false;
        }
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        if !self.connected_rx {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "test server is not connected",
            )));
        }

        let input = self.buffers.input_append_buf();
        let mut buf = match self.rx.recv_timeout(timeout.after) {
            Ok(v) => v,
            Err(RecvTimeoutError::Timeout) => return Err(Error::Timeout(timeout.reason)),
            Err(RecvTimeoutError::Disconnected) => {
                trace!("Test server disconnected");
                self.connected_rx = false;
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "test server disconnected",
                )));
            }
        };

        let maybe_hangup = buf
            .windows(HANGUP.len())
            .enumerate()
            .find(|(_, w)| *w == HANGUP)
            .map(|(pos, _)| pos);

        if let Some(pos) = maybe_hangup {
            debug!("TEST: Found <hangup>");
            buf.drain(pos..);
            self.connected_rx = false;
        }

        assert!(input.len() >= buf.len());
        let max = input.len().min(buf.len());
        input[..max].copy_from_slice(&buf[..]);
        self.buffers.input_appended(max);
        Ok(max > 0)
    }

    fn is_open(&mut self) -> bool {
        self.connected_tx
    }

    fn is_tls(&self) -> bool {
        // Pretend this is tls to not get TLS wrappers
        true
    }
}

const HANGUP: &[u8] = b"<hangup>";

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
