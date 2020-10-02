use std::fmt;
use std::io::Read;
use std::result::Result;
use std::sync::{Arc, Mutex};
use std::time;

#[cfg(any(feature = "tls", feature = "native-tls"))]
use lazy_static::lazy_static;
use qstring::QString;
use url::{form_urlencoded, Url};

use crate::agent::{self, Agent, AgentState};
use crate::body::BodySize;
use crate::body::{Payload, SizedReader};
use crate::error::Error;
use crate::header::{self, Header};
use crate::unit::{self, Unit};
use crate::Response;

#[cfg(feature = "json")]
use super::SerdeValue;

/// Request instances are builders that creates a request.
///
/// ```
/// let mut request = ureq::get("https://www.google.com/");
///
/// let response = request
///     .query("foo", "bar baz") // add ?foo=bar%20baz
///     .call();                 // run the request
/// ```
#[derive(Clone, Default)]
pub struct Request {
    pub(crate) agent: Arc<Mutex<AgentState>>,

    // via agent
    pub(crate) method: String,
    url: String,

    // from request itself
    pub(crate) headers: Vec<Header>,
    pub(crate) query: QString,
    pub(crate) timeout_connect: u64,
    pub(crate) timeout_read: u64,
    pub(crate) timeout_write: u64,
    pub(crate) timeout: Option<time::Duration>,
    pub(crate) redirects: u32,
    pub(crate) proxy: Option<crate::proxy::Proxy>,
    #[cfg(feature = "tls")]
    pub(crate) tls_config: Option<agent::TLSClientConfig>,
    #[cfg(all(feature = "native-tls", not(feature = "tls")))]
    pub(crate) tls_connector: Option<agent::TLSConnector>,
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (path, query) = self
            .to_url()
            .map(|u| {
                let query = unit::combine_query(&u, &self.query, true);
                (u.path().to_string(), query)
            })
            .unwrap_or_else(|_| ("BAD_URL".to_string(), "BAD_URL".to_string()));
        write!(
            f,
            "Request({} {}{}, {:?})",
            self.method, path, query, self.headers
        )
    }
}

impl Request {
    pub(crate) fn new(agent: &Agent, method: String, url: String) -> Request {
        Request {
            agent: Arc::clone(&agent.state),
            method,
            url,
            headers: agent.headers.clone(),
            redirects: 5,
            ..Default::default()
        }
    }

    /// "Builds" this request which is effectively the same as cloning.
    /// This is needed when we use a chain of request builders, but
    /// don't want to send the request at the end of the chain.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .set("X-Foo-Bar", "Baz")
    ///     .build();
    /// ```
    pub fn build(&self) -> Request {
        self.clone()
    }

    /// Executes the request and blocks the caller until done.
    ///
    /// Use `.timeout_connect()` and `.timeout_read()` to avoid blocking forever.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .timeout_connect(10_000) // max 10 seconds
    ///     .call();
    ///
    /// println!("{:?}", r);
    /// ```
    pub fn call(&mut self) -> Response {
        self.do_call(Payload::Empty)
    }

    fn do_call(&mut self, payload: Payload) -> Response {
        self.to_url()
            .and_then(|url| {
                let reader = payload.into_read();
                let unit = Unit::new(&self, &url, true, &reader);
                unit::connect(&self, unit, true, 0, reader, false)
            })
            .unwrap_or_else(|e| e.into())
    }

    /// Send data a json value.
    ///
    /// Requires feature `ureq = { version = "*", features = ["json"] }`
    ///
    /// The `Content-Length` header is implicitly set to the length of the serialized value.
    ///
    /// ```
    /// #[macro_use]
    /// extern crate ureq;
    ///
    /// fn main() {
    /// let r = ureq::post("/my_page")
    ///     .send_json(json!({ "name": "martin", "rust": true }));
    /// println!("{:?}", r);
    /// }
    /// ```
    #[cfg(feature = "json")]
    pub fn send_json(&mut self, data: SerdeValue) -> Response {
        if self.header("Content-Type").is_none() {
            self.set("Content-Type", "application/json");
        }
        self.do_call(Payload::JSON(data))
    }

    /// Send data as bytes.
    ///
    /// The `Content-Length` header is implicitly set to the length of the serialized value.
    ///
    /// ```
    /// let body = b"Hello world!";
    /// let r = ureq::post("/my_page")
    ///     .send_bytes(body);
    /// println!("{:?}", r);
    /// ```
    pub fn send_bytes(&mut self, data: &[u8]) -> Response {
        self.do_call(Payload::Bytes(data.to_owned()))
    }

    /// Send data as a string.
    ///
    /// The `Content-Length` header is implicitly set to the length of the serialized value.
    /// Defaults to `utf-8`
    ///
    /// ## Charset support
    ///
    /// Requires feature `ureq = { version = "*", features = ["charset"] }`
    ///
    /// If a `Content-Type` header is present and it contains a charset specification, we
    /// attempt to encode the string using that character set. If it fails, we fall back
    /// on utf-8.
    ///
    /// ```
    /// // this example requires features = ["charset"]
    ///
    /// let r = ureq::post("/my_page")
    ///     .set("Content-Type", "text/plain; charset=iso-8859-1")
    ///     .send_string("Hällo Wörld!");
    /// println!("{:?}", r);
    /// ```
    pub fn send_string(&mut self, data: &str) -> Response {
        let text = data.into();
        let charset =
            crate::response::charset_from_content_type(self.header("content-type")).to_string();
        self.do_call(Payload::Text(text, charset))
    }

    /// Send a sequence of (key, value) pairs as form-urlencoded data.
    ///
    /// The `Content-Type` header is implicitly set to application/x-www-form-urlencoded.
    /// The `Content-Length` header is implicitly set to the length of the serialized value.
    ///
    /// ```
    /// #[macro_use]
    /// extern crate ureq;
    ///
    /// fn main() {
    /// let r = ureq::post("/my_page")
    ///     .send_form(&[("foo", "bar"),("foo2", "bar2")]);
    /// println!("{:?}", r);
    /// }
    /// ```
    pub fn send_form(&mut self, data: &[(&str, &str)]) -> Response {
        if self.header("Content-Type").is_none() {
            self.set("Content-Type", "application/x-www-form-urlencoded");
        }
        let encoded = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(data)
            .finish();
        self.do_call(Payload::Bytes(encoded.into_bytes()))
    }

    /// Send data from a reader.
    ///
    /// If no Content-Length and Transfer-Encoding header has been set, it uses the [chunked transfer encoding](https://tools.ietf.org/html/rfc7230#section-4.1).
    ///
    /// The caller may set the Content-Length header to the expected byte size of the reader if is
    /// known.
    ///
    /// The input from the reader is buffered into chunks of size 16,384, the max size of a TLS fragment.
    ///
    /// ```
    /// use std::io::Cursor;
    ///
    /// let read = Cursor::new(vec![0x20; 100_000]);
    ///
    /// let resp = ureq::post("http://localhost/example-upload")
    ///     .set("Content-Type", "text/plain")
    ///     .send(read);
    /// ```
    pub fn send(&mut self, reader: impl Read + 'static) -> Response {
        self.do_call(Payload::Reader(Box::new(reader)))
    }

    /// Set a header field.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .set("Accept", "text/plain")
    ///     .call();
    ///
    ///  if r.ok() {
    ///      println!("yay got {}", r.into_string().unwrap());
    ///  } else {
    ///      println!("Oh no error!");
    ///  }
    /// ```
    pub fn set(&mut self, header: &str, value: &str) -> &mut Request {
        header::add_header(&mut self.headers, Header::new(header, value));
        self
    }

    /// Returns the value for a set header.
    ///
    /// ```
    /// let req = ureq::get("/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .build();
    /// assert_eq!("foobar", req.header("x-api-Key").unwrap());
    /// ```
    pub fn header(&self, name: &str) -> Option<&str> {
        header::get_header(&self.headers, name)
    }

    /// A list of the set header names in this request. Lowercased to be uniform.
    ///
    /// ```
    /// let req = ureq::get("/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .set("Content-Type", "application/json")
    ///     .build();
    /// assert_eq!(req.header_names(), vec!["x-api-key", "content-type"]);
    /// ```
    pub fn header_names(&self) -> Vec<String> {
        self.headers
            .iter()
            .map(|h| h.name().to_ascii_lowercase())
            .collect()
    }

    /// Tells if the header has been set.
    ///
    /// ```
    /// let req = ureq::get("/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .build();
    /// assert_eq!(true, req.has("x-api-Key"));
    /// ```
    pub fn has(&self, name: &str) -> bool {
        header::has_header(&self.headers, name)
    }

    /// All headers corresponding values for the give name, or empty vector.
    ///
    /// ```
    /// let req = ureq::get("/my_page")
    ///     .set("X-Forwarded-For", "1.2.3.4")
    ///     .set("X-Forwarded-For", "2.3.4.5")
    ///     .build();
    /// assert_eq!(req.all("x-forwarded-for"), vec![
    ///     "1.2.3.4",
    ///     "2.3.4.5",
    /// ]);
    /// ```
    pub fn all(&self, name: &str) -> Vec<&str> {
        header::get_all_headers(&self.headers, name)
    }

    /// Set a query parameter.
    ///
    /// For example, to set `?format=json&dest=/login`
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .query("format", "json")
    ///     .query("dest", "/login")
    ///     .call();
    ///
    /// println!("{:?}", r);
    /// ```
    pub fn query(&mut self, param: &str, value: &str) -> &mut Request {
        self.query.add_pair((param, value));
        self
    }

    /// Set query parameters as a string.
    ///
    /// For example, to set `?format=json&dest=/login`
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .query_str("?format=json&dest=/login")
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn query_str(&mut self, query: &str) -> &mut Request {
        self.query.add_str(query);
        self
    }

    /// Timeout for the socket connection to be successful.
    /// If both this and .timeout() are both set, .timeout_connect()
    /// takes precedence.
    ///
    /// The default is `0`, which means a request can block forever.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .timeout_connect(1_000) // wait max 1 second to connect
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn timeout_connect(&mut self, millis: u64) -> &mut Request {
        self.timeout_connect = millis;
        self
    }

    /// Timeout for the individual reads of the socket.
    /// If both this and .timeout() are both set, .timeout()
    /// takes precedence.
    ///
    /// The default is `0`, which means it can block forever.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .timeout_read(1_000) // wait max 1 second for the read
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn timeout_read(&mut self, millis: u64) -> &mut Request {
        self.timeout_read = millis;
        self
    }

    /// Timeout for the individual writes to the socket.
    /// If both this and .timeout() are both set, .timeout()
    /// takes precedence.
    ///
    /// The default is `0`, which means it can block forever.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .timeout_write(1_000)   // wait max 1 second for sending.
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn timeout_write(&mut self, millis: u64) -> &mut Request {
        self.timeout_write = millis;
        self
    }

    /// Timeout for the overall request, including DNS resolution, connection
    /// time, redirects, and reading the response body. Slow DNS resolution
    /// may cause a request to exceed the timeout, because the DNS request
    /// cannot be interrupted with the available APIs.
    ///
    /// This takes precedence over .timeout_read() and .timeout_write(), but
    /// not .timeout_connect().
    ///
    /// ```
    /// // wait max 1 second for whole request to complete.
    /// let r = ureq::get("/my_page")
    ///     .timeout(std::time::Duration::from_secs(1))
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn timeout(&mut self, timeout: time::Duration) -> &mut Request {
        self.timeout = Some(timeout);
        self
    }

    /// Basic auth.
    ///
    /// These are the same
    ///
    /// ```
    /// let r1 = ureq::get("http://localhost/my_page")
    ///     .auth("martin", "rubbermashgum")
    ///     .call();
    ///  println!("{:?}", r1);
    ///
    /// let r2 = ureq::get("http://martin:rubbermashgum@localhost/my_page").call();
    /// println!("{:?}", r2);
    /// ```
    pub fn auth(&mut self, user: &str, pass: &str) -> &mut Request {
        let pass = agent::basic_auth(user, pass);
        self.auth_kind("Basic", &pass)
    }

    /// Auth of other kinds such as `Digest`, `Token` etc.
    ///
    /// ```
    /// let r = ureq::get("http://localhost/my_page")
    ///     .auth_kind("token", "secret")
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn auth_kind(&mut self, kind: &str, pass: &str) -> &mut Request {
        let value = format!("{} {}", kind, pass);
        self.set("Authorization", &value);
        self
    }

    /// How many redirects to follow.
    ///
    /// Defaults to `5`. Set to `0` to avoid redirects and instead
    /// get a response object with the 3xx status code.
    ///
    /// If the redirect count hits this limit (and it's > 0), a synthetic 500 error
    /// response is produced.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .redirects(10)
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn redirects(&mut self, n: u32) -> &mut Request {
        self.redirects = n;
        self
    }

    /// Get the method this request is using.
    ///
    /// Example:
    /// ```
    /// let req = ureq::post("/somewhere")
    ///     .build();
    /// assert_eq!(req.get_method(), "POST");
    /// ```
    pub fn get_method(&self) -> &str {
        &self.method
    }

    /// Get the url this request was created with.
    ///
    /// This value is not normalized, it is exactly as set.
    /// It does not contain any added query parameters.
    ///
    /// Example:
    /// ```
    /// let req = ureq::post("https://cool.server/innit")
    ///     .build();
    /// assert_eq!(req.get_url(), "https://cool.server/innit");
    /// ```
    pub fn get_url(&self) -> &str {
        &self.url
    }

    /// Normalizes and returns the host that will be used for this request.
    ///
    /// Example:
    /// ```
    /// let req1 = ureq::post("https://cool.server/innit")
    ///     .build();
    /// assert_eq!(req1.get_host().unwrap(), "cool.server");
    ///
    /// let req2 = ureq::post("http://localhost/some/path")
    ///     .build();
    /// assert_eq!(req2.get_host().unwrap(), "localhost");
    /// ```
    pub fn get_host(&self) -> Result<String, Error> {
        match self.to_url() {
            Ok(u) => match u.host_str() {
                Some(host) => Ok(host.to_string()),
                None => Err(Error::BadUrl("No hostname in URL".into())),
            },
            Err(e) => Err(e),
        }
    }

    /// Returns the scheme for this request.
    ///
    /// Example:
    /// ```
    /// let req = ureq::post("https://cool.server/innit")
    ///     .build();
    /// assert_eq!(req.get_scheme().unwrap(), "https");
    /// ```
    pub fn get_scheme(&self) -> Result<String, Error> {
        self.to_url().map(|u| u.scheme().to_string())
    }

    /// The complete query for this request.
    ///
    /// Example:
    /// ```
    /// let req = ureq::post("https://cool.server/innit?foo=bar")
    ///     .query("format", "json")
    ///     .build();
    /// assert_eq!(req.get_query().unwrap(), "?foo=bar&format=json");
    /// ```
    pub fn get_query(&self) -> Result<String, Error> {
        self.to_url()
            .map(|u| unit::combine_query(&u, &self.query, true))
    }

    /// The normalized url of this request.
    ///
    /// Example:
    /// ```
    /// let req = ureq::post("https://cool.server/innit")
    ///     .build();
    /// assert_eq!(req.get_path().unwrap(), "/innit");
    /// ```
    pub fn get_path(&self) -> Result<String, Error> {
        self.to_url().map(|u| u.path().to_string())
    }

    fn to_url(&self) -> Result<Url, Error> {
        Url::parse(&self.url).map_err(|e| Error::BadUrl(format!("{}", e)))
    }

    /// Set the proxy server to use for the connection.
    ///
    /// Example:
    /// ```
    /// let proxy = ureq::Proxy::new("user:password@cool.proxy:9090").unwrap();
    /// let req = ureq::post("https://cool.server")
    ///     .set_proxy(proxy)
    ///     .build();
    /// ```
    pub fn set_proxy(&mut self, proxy: crate::proxy::Proxy) -> &mut Request {
        self.proxy = Some(proxy);
        self
    }

    /// Set the TLS client config to use for the connection.
    ///
    /// See [`ClientConfig`](https://docs.rs/rustls/latest/rustls/struct.ClientConfig.html).
    ///
    /// Example:
    /// ```
    /// let tls_config = std::sync::Arc::new(rustls::ClientConfig::new());
    /// let req = ureq::post("https://cool.server")
    ///     .set_tls_config(tls_config.clone());
    /// ```
    #[cfg(feature = "tls")]
    pub fn set_tls_config(&mut self, tls_config: Arc<rustls::ClientConfig>) -> &mut Request {
        self.tls_config = Some(agent::TLSClientConfig(tls_config));
        self
    }

    #[cfg(feature = "tls")]
    pub(crate) fn tls_config(&self) -> Arc<rustls::ClientConfig> {
        lazy_static! {
            static ref TLS_CONF: Arc<rustls::ClientConfig> = {
                let mut config = rustls::ClientConfig::new();
                configure_certs(&mut config);
                Arc::new(config)
            };
        }

        self.tls_config
            .as_ref()
            .or(self.agent.lock().unwrap().tls_config.as_ref())
            .as_ref()
            .map(|c| c.0.clone())
            .unwrap_or(TLS_CONF.clone())
    }

    #[cfg(feature = "native-tls")]
    pub(crate) fn tls_connector(&self) -> Option<Arc<native_tls::TlsConnector>> {
        lazy_static! {
            static ref TLS_CONNECTOR: Option<Arc<native_tls::TlsConnector>> = {
                match native_tls::TlsConnector::new() {
                    Ok(tc) => Some(Arc::new(tc)),
                    _ => None,
                }
            };
        }
        self.tls_connector
            .as_ref()
            .or(self.agent.lock().unwrap().tls_connector.as_ref())
            .as_ref()
            .map(|c| c.0.clone())
            .or(TLS_CONNECTOR.clone())
    }

    /// Sets the TLS connector that will be used for the connection.
    ///
    /// Example:
    /// ```
    /// let tls_connector = std::sync::Arc::new(native_tls::TlsConnector::new().unwrap());
    /// let req = ureq::post("https://cool.server")
    ///     .set_tls_connector(tls_connector.clone());
    /// ```
    #[cfg(all(feature = "native-tls", not(feature = "tls")))]
    pub fn set_tls_connector(
        &mut self,
        tls_connector: Arc<native_tls::TlsConnector>,
    ) -> &mut Request {
        self.tls_connector = Some(agent::TLSConnector(tls_connector));
        self
    }

    // Returns true if this request, with the provided body, is retryable.
    pub(crate) fn is_retryable(&self, body: &SizedReader) -> bool {
        // Per https://tools.ietf.org/html/rfc7231#section-8.1.3
        // these methods are idempotent.
        let idempotent = match self.method.as_str() {
            "DELETE" | "GET" | "HEAD" | "OPTIONS" | "PUT" | "TRACE" => true,
            _ => false,
        };
        // Unsized bodies aren't retryable because we can't rewind the reader.
        // Sized bodies are retryable only if they are zero-length because of
        // coincidences of the current implementation - the function responsible
        // for retries doesn't have a way to replay a Payload.
        let retryable_body = match body.size {
            BodySize::Unknown => false,
            BodySize::Known(0) => true,
            BodySize::Known(_) => false,
            BodySize::Empty => true,
        };

        idempotent && retryable_body
    }
}

#[cfg(all(feature = "tls", feature = "native-certs"))]
fn configure_certs(config: &mut rustls::ClientConfig) {
    config.root_store =
        rustls_native_certs::load_native_certs().expect("Could not load patform certs");
}

#[cfg(all(feature = "tls", not(feature = "native-certs")))]
fn configure_certs(config: &mut rustls::ClientConfig) {
    config
        .root_store
        .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
}

#[test]
fn no_hostname() {
    let req = Request::new(
        &Agent::default(),
        "GET".to_string(),
        "unix:/run/foo.socket".to_string(),
    );
    assert!(req.get_host().is_err());
}
