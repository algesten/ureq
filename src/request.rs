use std::fmt;
use std::io::Read;

use qstring::QString;
use url::{form_urlencoded, Url};

use crate::agent::Agent;
use crate::body::BodySize;
use crate::body::{Payload, SizedReader};
use crate::error::Error;
use crate::header::{self, Header};
use crate::proxy::Proxy;
use crate::unit::{self, Unit};
use crate::Response;

#[cfg(feature = "json")]
use super::SerdeValue;

pub type Result<T> = std::result::Result<T, Error>;

/// Request instances are builders that creates a request.
///
/// ```
/// let mut request = ureq::get("https://www.google.com/");
///
/// let response = request
///     .query("foo", "bar baz") // add ?foo=bar%20baz
///     .call();                 // run the request
/// ```
#[derive(Clone)]
pub struct Request {
    pub(crate) agent: Agent,
    pub(crate) method: String,
    url: String,
    return_error_for_status: bool,
    pub(crate) headers: Vec<Header>,
    pub(crate) query: QString,
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
    pub(crate) fn new(agent: Agent, method: String, url: String) -> Request {
        Request {
            agent,
            method,
            url,
            headers: vec![],
            return_error_for_status: true,
            query: QString::default(),
        }
    }

    /// Executes the request and blocks the caller until done.
    ///
    /// Use `.timeout_connect()` and `.timeout_read()` to avoid blocking forever.
    ///
    /// ```
    /// let r = ureq::builder()
    ///     .timeout_connect(std::time::Duration::from_secs(10)) // max 10 seconds
    ///     .build()
    ///     .get("/my_page")
    ///     .call();
    ///
    /// println!("{:?}", r);
    /// ```
    pub fn call(self) -> Result<Response> {
        self.do_call(Payload::Empty)
    }

    fn do_call(&self, payload: Payload) -> Result<Response> {
        for h in &self.headers {
            h.validate()?;
        }
        let response = self.to_url().and_then(|url| {
            let reader = payload.into_read();
            let unit = Unit::new(&self, &url, true, &reader);
            unit::connect(&self, unit, true, 0, reader, false)
        })?;

        if response.error() && self.return_error_for_status {
            Err(Error::HTTP(response.into()))
        } else {
            Ok(response)
        }
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
    pub fn send_json(mut self, data: SerdeValue) -> Result<Response> {
        if self.header("Content-Type").is_none() {
            self = self.set("Content-Type", "application/json");
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
    pub fn send_bytes(self, data: &[u8]) -> Result<Response> {
        self.do_call(Payload::Bytes(data))
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
    pub fn send_string(self, data: &str) -> Result<Response> {
        let charset =
            crate::response::charset_from_content_type(self.header("content-type")).to_string();
        self.do_call(Payload::Text(data, charset))
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
    pub fn send_form(mut self, data: &[(&str, &str)]) -> Result<Response> {
        if self.header("Content-Type").is_none() {
            self = self.set("Content-Type", "application/x-www-form-urlencoded");
        }
        let encoded = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(data)
            .finish();
        self.do_call(Payload::Bytes(&encoded.into_bytes()))
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
    pub fn send(self, reader: impl Read) -> Result<Response> {
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
    ///  if r.is_ok() {
    ///      println!("yay got {}", r.unwrap().into_string().unwrap());
    ///  } else {
    ///      println!("Oh no error!");
    ///  }
    /// ```
    pub fn set(mut self, header: &str, value: &str) -> Self {
        header::add_header(&mut self.headers, Header::new(header, value));
        self
    }

    /// Returns the value for a set header.
    ///
    /// ```
    /// let req = ureq::get("/my_page")
    ///     .set("X-API-Key", "foobar");
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
    ///     .set("Content-Type", "application/json");
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
    ///     .set("X-API-Key", "foobar");
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
    ///     .set("X-Forwarded-For", "2.3.4.5");
    ///
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
    pub fn query(mut self, param: &str, value: &str) -> Self {
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
    pub fn query_str(mut self, query: &str) -> Self {
        self.query.add_str(query);
        self
    }

    /// By default, if a response's status is anything but a 2xx or 3xx,
    /// send() and related methods will return an Error. If you want
    /// to handle such responses as non-errors, set this to false.
    ///
    /// Example:
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// let result = ureq::get("http://httpbin.org/status/500")
    ///     .error_for_status(false)
    ///     .call();
    /// assert!(result.is_ok());
    /// # Ok(())
    /// # }
    /// ```
    pub fn error_for_status(mut self, value: bool) -> Self {
        self.return_error_for_status = value;
        self
    }

    /// Get the method this request is using.
    ///
    /// Example:
    /// ```
    /// let req = ureq::post("/somewhere");
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
    /// let req = ureq::post("https://cool.server/innit");
    /// assert_eq!(req.get_url(), "https://cool.server/innit");
    /// ```
    pub fn get_url(&self) -> &str {
        &self.url
    }

    /// Normalizes and returns the host that will be used for this request.
    ///
    /// Example:
    /// ```
    /// let req1 = ureq::post("https://cool.server/innit");
    /// assert_eq!(req1.get_host().unwrap(), "cool.server");
    ///
    /// let req2 = ureq::post("http://localhost/some/path");
    /// assert_eq!(req2.get_host().unwrap(), "localhost");
    /// ```
    pub fn get_host(&self) -> Result<String> {
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
    /// let req = ureq::post("https://cool.server/innit");
    /// assert_eq!(req.get_scheme().unwrap(), "https");
    /// ```
    pub fn get_scheme(&self) -> Result<String> {
        self.to_url().map(|u| u.scheme().to_string())
    }

    /// The complete query for this request.
    ///
    /// Example:
    /// ```
    /// let req = ureq::post("https://cool.server/innit?foo=bar")
    ///     .query("format", "json");
    /// assert_eq!(req.get_query().unwrap(), "?foo=bar&format=json");
    /// ```
    pub fn get_query(&self) -> Result<String> {
        self.to_url()
            .map(|u| unit::combine_query(&u, &self.query, true))
    }

    /// The normalized url of this request.
    ///
    /// Example:
    /// ```
    /// let req = ureq::post("https://cool.server/innit");
    /// assert_eq!(req.get_path().unwrap(), "/innit");
    /// ```
    pub fn get_path(&self) -> Result<String> {
        self.to_url().map(|u| u.path().to_string())
    }

    fn to_url(&self) -> Result<Url> {
        Url::parse(&self.url).map_err(|e| Error::BadUrl(format!("{}", e)))
    }

    pub(crate) fn proxy(&self) -> Option<Proxy> {
        if let Some(proxy) = &self.agent.config.proxy {
            Some(proxy.clone())
        } else {
            None
        }
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

#[test]
fn no_hostname() {
    let req = Request::new(
        Agent::new(),
        "GET".to_string(),
        "unix:/run/foo.socket".to_string(),
    );
    assert!(req.get_host().is_err());
}

#[test]
fn request_implements_send_and_sync() {
    let _request: Box<dyn Send> = Box::new(Request::new(
        Agent::new(),
        "GET".to_string(),
        "https://example.com/".to_string(),
    ));
    let _request: Box<dyn Sync> = Box::new(Request::new(
        Agent::new(),
        "GET".to_string(),
        "https://example.com/".to_string(),
    ));
}

#[test]
fn send_byte_slice() {
    let bytes = vec![1, 2, 3];
    crate::agent()
        .post("http://example.com")
        .send(&bytes[1..2])
        .ok();
}
