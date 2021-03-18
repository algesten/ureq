use std::io::Read;
use std::{fmt, time};

use url::{form_urlencoded, Url};

use crate::header::{self, Header};
use crate::unit::{self, Unit};
use crate::Response;
use crate::{agent::Agent, error::Error};
use crate::{body::Payload, ErrorKind};

#[cfg(feature = "json")]
use super::SerdeValue;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
struct ParsedUrl(std::result::Result<Url, url::ParseError>);

impl fmt::Display for ParsedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(url) = &self.0 {
            write!(f, "{}", url.as_str())
        } else {
            write!(f, "{:?}", self.0)
        }
    }
}

impl From<String> for ParsedUrl {
    fn from(s: String) -> Self {
        ParsedUrl(s.parse())
    }
}

impl From<Url> for ParsedUrl {
    fn from(url: Url) -> Self {
        ParsedUrl(Ok(url))
    }
}

// RequestUrl is a read-only wrapper around a URL that is ready to be used as
// part of an HTTP request. It provides more convenient accessors than
// url::Url. For instance, it returns `&str`'s for query_pairs rather than
// Cow<'a, str>, and it it provides `&str` rather than `Option<&str>` for
// host, because a non-empty host is always required for a request.
pub struct RequestUrl {
    url: Url,
    query_pairs: Vec<(String, String)>,
}

impl RequestUrl {
    fn new(url: Url) -> Self {
        let query_pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        Self { url, query_pairs }
    }

    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    pub fn scheme(&self) -> &str {
        self.url.scheme()
    }

    pub fn host(&self) -> &str {
        self.url.host_str().unwrap()
    }

    pub fn port(&self) -> Option<u16> {
        self.url.port()
    }

    pub fn path(&self) -> &str {
        self.url.path()
    }

    pub fn query_pairs(&self) -> Vec<(&str, &str)> {
        self.query_pairs
            .iter()
            .map(|qp| (qp.0.as_str(), qp.1.as_str()))
            .collect()
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
    parsed_url: ParsedUrl,
    error_on_non_2xx: bool,
    headers: Vec<Header>,
    timeout: Option<time::Duration>,
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Request({} {}, {:?})",
            self.method, self.parsed_url, self.headers
        )
    }
}

impl Request {
    pub(crate) fn new(agent: Agent, method: String, url: String) -> Request {
        Self::_new(agent, method, url.into())
    }

    pub(crate) fn with_url(agent: Agent, method: String, url: Url) -> Request {
        Self::_new(agent, method, url.into())
    }

    fn _new(agent: Agent, method: String, parsed_url: ParsedUrl) -> Request {
        Request {
            agent,
            method,
            parsed_url,
            headers: vec![],
            error_on_non_2xx: true,
            timeout: None,
        }
    }

    #[inline(always)]
    /// Sets overall timeout for the request, overriding agent's configuration if any.
    pub fn timeout(mut self, timeout: time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
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

    fn do_call(self, payload: Payload) -> Result<Response> {
        for h in &self.headers {
            h.validate()?;
        }
        let url = self.parsed_url.0?;

        let deadline = match self.timeout.or(self.agent.config.timeout) {
            None => None,
            Some(timeout) => {
                let now = time::Instant::now();
                Some(now.checked_add(timeout).unwrap())
            }
        };

        let reader = payload.into_read();
        let unit = Unit::new(
            &self.agent,
            &self.method,
            &url,
            &self.headers,
            &reader,
            deadline,
        );
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
    /// let resp = ureq::get("http://httpbin.org/get")
    ///     .query("format", "json")
    ///     .query("dest", "/login")
    ///     .call()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn query(mut self, param: &str, value: &str) -> Self {
        if let Ok(url) = &mut self.parsed_url.0 {
            url.query_pairs_mut().append_pair(param, value);
        }
        self
    }

    /// Returns the value of the request method. Something like `GET`, `POST`, `PUT` etc.
    ///
    /// ```
    /// let req = ureq::put("http://httpbin.org/put");
    ///
    /// assert_eq!(req.method(), "PUT");
    /// ```
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Returns the RequestUrl that will be used to perform this request.
    /// This can be used, for instance, to access the parsed components of
    /// the URL, like path and query parameters.
    ///
    /// If this Request was built with a URL that fails to parse or doesn't
    /// contain a host, returns Err.
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let req = ureq::get("http://httpbin.org/get")
    ///     .query("foo", "42")
    ///     .query("foo", "43");
    ///
    /// let rurl: ureq::RequestUrl = req.get_url()?;
    /// assert_eq!(rurl.scheme(), "http");
    /// assert_eq!(rurl.host(), "httpbin.org");
    /// assert_eq!(rurl.port(), None);
    /// assert_eq!(rurl.path(), "/get");
    /// assert_eq!(rurl.query_pairs(), vec![
    ///     ("foo", "42"),
    ///     ("foo", "43")
    /// ]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_url(&self) -> Result<RequestUrl> {
        match self.parsed_url.clone().0 {
            Ok(url) => {
                if url.host().is_some() {
                    Ok(RequestUrl::new(url.clone()))
                } else {
                    Err(ErrorKind::InvalidUrl.msg("Invalid URL: no hostname"))
                }
            }
            Err(parse_error) => Err(ErrorKind::InvalidUrl.msg("Invalid URL").src(parse_error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
