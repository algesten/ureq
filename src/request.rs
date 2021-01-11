use std::fmt;
use std::io::Read;

use url::{form_urlencoded, Url};

use crate::body::Payload;
use crate::error::ErrorKind;
use crate::header::{self, Header};
use crate::unit::{self, Unit};
use crate::Response;
use crate::{agent::Agent, error::Error};

#[cfg(feature = "json")]
use super::SerdeValue;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
enum Urlish {
    Url(Url),
    Str(String),
}

impl fmt::Display for Urlish {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Urlish::Url(u) => write!(f, "{}", u),
            Urlish::Str(s) => write!(f, "{}", s),
        }
    }
}

/// Request instances are builders that creates a request.
///
/// ```
/// # fn main() -> Result<(), ureq::Error> {
/// # ureq::is_test(true);
/// let response = ureq::get("http://example.com/form")
///     .query("foo", "bar baz")  // add ?foo=bar+baz
///     .call()?;                 // run the request
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Request {
    agent: Agent,
    method: String,
    url: Urlish,
    error_on_non_2xx: bool,
    headers: Vec<Header>,
    query_params: Vec<(String, String)>,
}

/// Format: `$method $url`.
/// ```
/// let request = ureq::get("http://example.com/form")
///     .query("foo", "bar baz");  // not included in Display
///
/// assert_eq!(format!("{}", request), "GET http://example.com/form");
/// ```
impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.method, self.url)
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Request({} {} {:?}, {:?})",
            self.method, self.url, self.query_params, self.headers
        )
    }
}

impl Request {
    pub(crate) fn new(agent: Agent, method: String, url: String) -> Request {
        Request {
            agent,
            method,
            url: Urlish::Str(url),
            headers: vec![],
            error_on_non_2xx: true,
            query_params: vec![],
        }
    }

    pub(crate) fn with_url(agent: Agent, method: String, url: Url) -> Request {
        Request {
            agent,
            method,
            url: Urlish::Url(url),
            headers: vec![],
            error_on_non_2xx: true,
            query_params: vec![],
        }
    }

    /// Sends the request with no body and blocks the caller until done.
    ///
    /// Use this with GET, HEAD, OPTIONS or TRACE. It sends neither
    /// Content-Length nor Transfer-Encoding.
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::get("http://example.com/")
    ///     .call()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn call(self) -> Result<Response> {
        self.do_call(Payload::Empty)
    }

    fn do_call(&self, payload: Payload) -> Result<Response> {
        for h in &self.headers {
            h.validate()?;
        }
        let mut url: Url = match self.url.clone() {
            Urlish::Url(u) => u,
            Urlish::Str(s) => s.parse().map_err(|e: url::ParseError| {
                ErrorKind::InvalidUrl
                    .msg(&format!("failed to parse URL '{}'", self.url))
                    .src(e)
            })?,
        };
        for (name, value) in self.query_params.clone() {
            url.query_pairs_mut().append_pair(&name, &value);
        }
        let reader = payload.into_read();
        let unit = Unit::new(&self.agent, &self.method, &url, &self.headers, &reader);
        let response = unit::connect(unit, true, reader).map_err(|e| e.url(url.clone()))?;

        if response.status() >= 400 {
            Err(Error::Status(response.status(), response))
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::post("http://httpbin.org/post")
    ///     .send_json(ureq::json!({
    ///       "name": "martin",
    ///       "rust": true,
    ///     }))?;
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::put("http://httpbin.org/put")
    ///     .send_bytes(&[0; 1000])?;
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::post("http://httpbin.org/post")
    ///     .set("Content-Type", "text/plain; charset=iso-8859-1")
    ///     .send_string("Hällo Wörld!")?;
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::post("http://httpbin.org/post")
    ///     .send_form(&[
    ///       ("foo", "bar"),
    ///       ("foo2", "bar2"),
    ///     ])?;
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let read = Cursor::new(vec![0x20; 100]);
    /// let resp = ureq::post("http://httpbin.org/post")
    ///     .send(read)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn send(self, reader: impl Read) -> Result<Response> {
        self.do_call(Payload::Reader(Box::new(reader)))
    }

    /// Set a header field.
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::get("http://httpbin.org/bytes/1000")
    ///     .set("Accept", "text/plain")
    ///     .set("Range", "bytes=500-999")
    ///     .call()?;
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::get("http://httpbin.org/response-headers")
    ///     .query("format", "json")
    ///     .query("dest", "/login")
    ///     .call()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn query(mut self, param: &str, value: &str) -> Self {
        self.query_params
            .push((param.to_string(), value.to_string()));
        self
    }
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
