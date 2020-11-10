use std::fmt;
use std::io::{self, Read};

use url::{form_urlencoded, Url};

use crate::agent::Agent;
use crate::body::BodySize;
use crate::body::{Payload, SizedReader};
use crate::error::Error;
use crate::header::{self, Header};
use crate::unit::{self, Unit};
use crate::Response;

#[cfg(feature = "json")]
use super::SerdeValue;

#[derive(Debug, Clone)]
enum Urlish {
    Url(Url),
    Str(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// RequestBuilder accumulates up the fields that make a [Request](struct.Request.html).
///
/// At least the method and URL must be set before calling `.build()`.
/// RequestBuilder has convenience methods `call()` and `send*()`, which
/// build a Request and then immediately perform the Request.
#[derive(Debug, Clone)]
pub struct RequestBuilder {
    agent: Option<Agent>,
    method: Option<String>,
    url: Option<Urlish>,
    return_error_for_status: bool,
    headers: Vec<Header>,
    query_params: Vec<(String, String)>,
}

/// A Request ready to be sent.
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
    pub(crate) url: Url,
    pub(crate) return_error_for_status: bool,
    pub(crate) headers: Vec<Header>,
}

impl RequestBuilder {
    /// Create a new RequestBuilder, with all fields empty.
    pub(crate) fn new() -> Self {
        RequestBuilder {
            agent: None,
            method: None,
            url: None,
            return_error_for_status: false,
            headers: vec![],
            query_params: vec![],
        }
    }

    /// Set the agent to be used by the Request built from this object.
    pub fn agent(mut self, agent: Agent) -> Self {
        self.agent = Some(agent);
        self
    }

    /// Set the method to be used by the Request built from this object.
    pub fn method(mut self, method: &str) -> Self {
        self.method = Some(method.to_string());
        self
    }

    /// Set the URL to send this request to. If you have an unparsed `String`
    /// or `&str` containing a URL, use `.url_str()` instead.
    pub fn url(mut self, url: Url) -> Self {
        self.url = Some(Urlish::Url(url));
        self
    }

    /// Set the URL to send this request to. If you have an already-parsed Url
    /// object, use `.url()` instead.
    pub fn url_str(mut self, url: &str) -> Self {
        self.url = Some(Urlish::Str(url.to_string()));
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

    /// Set a query parameter.
    ///
    /// This will be added to any query parameters already present in the URL.
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
        self.query_params
            .push((param.to_string(), value.to_string()));
        self
    }

    /// Consume this RequestBuilder and turn it into a Request. This may
    /// return an error if a URL provided as a `&str` fails to parse,
    /// headers were invalid, or the method or URL were unset.
    pub fn build(self) -> Result<Request> {
        let agent = self.agent.unwrap_or_else(|| crate::agent());
        let method = if let Some(method) = self.method {
            method
        } else {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::Other,
                "No method set",
            )));
        };
        let mut url: Url = match self.url {
            Some(Urlish::Url(u)) => u,
            Some(Urlish::Str(s)) => s
                .parse()
                .map_err(|e: url::ParseError| Error::BadUrl(e.to_string()))?,
            None => {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::Other,
                    "No URL set",
                )))
            }
        };
        for (name, value) in self.query_params {
            url.query_pairs_mut().append_pair(&name, &value);
        }
        for h in &self.headers {
            h.validate()?;
        }
        let req = Request {
            agent,
            method,
            url,
            return_error_for_status: self.return_error_for_status,
            headers: self.headers,
        };
        Ok(req)
    }

    /// Build a Request and send it with no body. See [Request.call](struct.Request.html#method.call).
    pub fn call(self) -> Result<Response> {
        self.build()?.call()
    }

    /// Build a Request and send it with a JSON body. See [Request.send_json](struct.Request.html#method.send_json).
    #[cfg(feature = "json")]
    pub fn send_json(self, data: SerdeValue) -> Result<Response> {
        self.build()?.send_json(data)
    }

    /// Build a Request and send it with bytes as the body. See [Request.send_bytes](struct.Request.html#method.send_bytes).
    pub fn send_bytes(self, data: &[u8]) -> Result<Response> {
        self.build()?.send_bytes(data)
    }

    /// Build a Request and send it with a string as the body. See [Request.send_string](struct.Request.html#method.send_string).
    pub fn send_string(self, data: &str) -> Result<Response> {
        self.build()?.send_string(data)
    }

    /// Build a Request and send it with a form as the body. See [Request.send_form](struct.Request.html#method.send_form).
    pub fn send_form(self, data: &[(&str, &str)]) -> Result<Response> {
        self.build()?.send_form(data)
    }

    /// Build a Request and send it with a body read from `reader`. See [Request.send](struct.Request.html#method.send).
    pub fn send(self, reader: impl Read) -> Result<Response> {
        self.build()?.send(reader)
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Request({} {}, {:?})",
            self.method, self.url, self.headers
        )
    }
}

impl Request {
    /// Returns the value for a set header.

    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// let req = ureq::get("http://example.com")
    ///     .set("X-API-Key", "foobar")
    ///     .build()?;

    /// assert_eq!("foobar", req.header("x-api-Key").unwrap());
    /// # Ok(())
    /// # }
    /// ```
    pub fn header(&self, name: &str) -> Option<&str> {
        header::get_header(&self.headers, name)
    }

    /// A list of the user-set header names in this request. Lowercased to be uniform.
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// let req = ureq::get("http://example.com/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .set("Content-Type", "application/json")
    ///     .build()?;
    /// assert_eq!(req.header_names(), vec!["x-api-key", "content-type"]);
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// let req = ureq::get("http://example.com/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .build()?;
    /// assert_eq!(true, req.has("x-api-Key"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn has(&self, name: &str) -> bool {
        header::has_header(&self.headers, name)
    }

    /// All headers corresponding values for the give name, or empty vector.
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// let req = ureq::get("http://example.com/my_page")
    ///     .set("X-Forwarded-For", "1.2.3.4")
    ///     .set("X-Forwarded-For", "2.3.4.5")
    ///     .build()?;
    ///
    /// assert_eq!(req.all("x-forwarded-for"), vec![
    ///     "1.2.3.4",
    ///     "2.3.4.5",
    /// ]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn all(&self, name: &str) -> Vec<&str> {
        header::get_all_headers(&self.headers, name)
    }

    /// Executes the request and blocks the caller until done.
    ///
    /// Use `.timeout_connect()` and `.timeout_read()` to avoid blocking forever.
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// let r = ureq::builder()
    ///     .timeout_connect(std::time::Duration::from_secs(10)) // max 10 seconds
    ///     .build()
    ///     .get("http://example.com/my_page")
    ///     .build()?
    ///     .call();
    ///
    /// println!("{:?}", r);
    /// # Ok(())
    /// # }
    /// ```
    pub fn call(self) -> Result<Response> {
        self.do_call(Payload::Empty)
    }

    fn do_call(&self, payload: Payload) -> Result<Response> {
        let reader = payload.into_read();
        let unit = Unit::new(&self, &reader);
        let response: Response = unit::connect(&self, unit, true, 0, reader, false)?;

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
            self.headers
                .push(Header::new("Content-Type", "application/json"));
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
            self.headers.push(Header::new(
                "Content-Type",
                "application/x-www-form-urlencoded",
            ));
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
fn request_implements_send_and_sync() {
    let _request: Box<dyn Send> = Box::new(
        RequestBuilder::new()
            .method("GET")
            .url_str("https://example.com/")
            .build()
            .unwrap(),
    );
    let _request: Box<dyn Sync> = Box::new(
        RequestBuilder::new()
            .method("GET")
            .url_str("https://example.com/")
            .build()
            .unwrap(),
    );
}

#[test]
fn send_byte_slice() {
    let bytes = vec![1, 2, 3];
    crate::agent()
        .post("http://example.com")
        .send(&bytes[1..2])
        .ok();
}
