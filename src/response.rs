use std::fmt;
use std::io::{self, Cursor, ErrorKind, Read};
use std::str::FromStr;

use chunked_transfer::Decoder as ChunkDecoder;

use crate::error::Error;
use crate::header::Header;
use crate::pool::PoolReturnRead;
use crate::stream::{DeadlineStream, Stream};
use crate::unit::Unit;

#[cfg(feature = "json")]
use serde::de::DeserializeOwned;

#[cfg(feature = "charset")]
use encoding::label::encoding_from_whatwg_label;
#[cfg(feature = "charset")]
use encoding::DecoderTrap;

pub const DEFAULT_CONTENT_TYPE: &str = "text/plain";
pub const DEFAULT_CHARACTER_SET: &str = "utf-8";

/// Response instances are created as results of firing off requests.
///
/// The `Response` is used to read response headers and decide what to do with the body.
/// Note that the socket connection is open and the body not read until one of
/// [`into_reader()`](#method.into_reader), [`into_json()`](#method.into_json),
/// [`into_json_deserialize()`](#method.into_json_deserialize) or
/// [`into_string()`](#method.into_string) consumes the response.
///
/// All error handling, including URL parse errors and connection errors, is done by mapping onto
/// [synthetic errors](#method.synthetic). Callers must check response.synthetic_error(),
/// response.is_ok(), or response.error() before relying on the contents of the reader.
///
/// ```
/// let response = ureq::get("https://www.google.com").call();
/// if let Some(error) = response.synthetic_error() {
///     eprintln!("{}", error);
///     return;
/// }
///
/// // socket is still open and the response body has not been read.
///
/// let text = response.into_string().unwrap();
///
/// // response is consumed, and body has been read.
/// ```
pub struct Response {
    url: Option<String>,
    error: Option<Error>,
    status_line: String,
    index: ResponseStatusIndex,
    status: u16,
    headers: Vec<Header>,
    unit: Option<Unit>,
    stream: Option<Stream>,
}

/// index into status_line where we split: HTTP/1.1 200 OK
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ResponseStatusIndex {
    http_version: usize,
    response_code: usize,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Response[status: {}, status_text: {}]",
            self.status(),
            self.status_text()
        )
    }
}

impl Response {
    /// Construct a response with a status, status text and a string body.
    ///
    /// This is hopefully useful for unit tests.
    ///
    /// Example:
    ///
    /// ```
    /// let resp = ureq::Response::new(401, "Authorization Required", "Please log in");
    ///
    /// assert_eq!(resp.status(), 401);
    /// ```
    pub fn new(status: u16, status_text: &str, body: &str) -> Self {
        let r = format!("HTTP/1.1 {} {}\r\n\r\n{}\n", status, status_text, body);
        (r.as_ref() as &str)
            .parse::<Response>()
            .unwrap_or_else(|e| e.into())
    }

    /// The URL we ended up at. This can differ from the request url when
    /// we have followed redirects.
    pub fn get_url(&self) -> &str {
        self.url.as_ref().map(|s| &s[..]).unwrap_or("")
    }

    /// The entire status line like: `HTTP/1.1 200 OK`
    pub fn status_line(&self) -> &str {
        self.status_line.as_str()
    }

    /// The http version: `HTTP/1.1`
    pub fn http_version(&self) -> &str {
        &self.status_line.as_str()[0..self.index.http_version]
    }

    /// The status as a u16: `200`
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The status text: `OK`
    pub fn status_text(&self) -> &str {
        &self.status_line.as_str()[self.index.response_code + 1..].trim()
    }

    /// The header corresponding header value for the give name, if any.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.is_name(name))
            .map(|h| h.value())
    }

    /// A list of the header names in this response.
    /// Lowercased to be uniform.
    pub fn headers_names(&self) -> Vec<String> {
        self.headers
            .iter()
            .map(|h| h.name().to_lowercase())
            .collect()
    }

    /// Tells if the response has the named header.
    pub fn has(&self, name: &str) -> bool {
        self.header(name).is_some()
    }

    /// All headers corresponding values for the give name, or empty vector.
    pub fn all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|h| h.is_name(name))
            .map(|h| h.value())
            .collect()
    }

    /// Whether the response status is: 200 <= status <= 299
    pub fn ok(&self) -> bool {
        self.status >= 200 && self.status <= 299
    }

    pub fn redirect(&self) -> bool {
        self.status >= 300 && self.status <= 399
    }

    /// Whether the response status is: 400 <= status <= 499
    pub fn client_error(&self) -> bool {
        self.status >= 400 && self.status <= 499
    }

    /// Whether the response status is: 500 <= status <= 599
    pub fn server_error(&self) -> bool {
        self.status >= 500 && self.status <= 599
    }

    /// Whether the response status is: 400 <= status <= 599
    pub fn error(&self) -> bool {
        self.client_error() || self.server_error()
    }

    /// Tells if this response is "synthetic".
    ///
    /// The [methods](struct.Request.html#method.call) [firing](struct.Request.html#method.send)
    /// [off](struct.Request.html#method.send_string)
    /// [requests](struct.Request.html#method.send_json)
    /// all return a `Response`; there is no rust style `Result`.
    ///
    /// Rather than exposing a custom error type through results, this library has opted
    /// for representing potential connection/TLS/etc errors as HTTP response codes.
    /// These invented codes are called "synthetic".
    ///
    /// The idea is that from a library user's point of view the distinction
    /// of whether a failure originated in the remote server (500, 502) etc, or some transient
    /// network failure, the code path of handling that would most often be the same.
    ///
    /// The specific mapping of error to code can be seen in the [`Error`](enum.Error.html) doc.
    ///
    /// However if the distinction is important, this method can be used to tell. Also see
    /// [synthetic_error()](struct.Response.html#method.synthetic_error)
    /// to see the actual underlying error.
    ///
    /// ```
    /// // scheme that this library doesn't understand
    /// let resp = ureq::get("borkedscheme://www.google.com").call();
    ///
    /// // it's an error
    /// assert!(resp.error());
    ///
    /// // synthetic error code 400
    /// assert_eq!(resp.status(), 400);
    ///
    /// // tell that it's synthetic.
    /// assert!(resp.synthetic());
    /// ```
    pub fn synthetic(&self) -> bool {
        self.error.is_some()
    }

    /// Get the actual underlying error when the response is
    /// ["synthetic"](struct.Response.html#method.synthetic).
    pub fn synthetic_error(&self) -> &Option<Error> {
        &self.error
    }

    // Internal-only API, to allow unit::connect to return early on errors.
    pub(crate) fn into_error(self) -> Option<Error> {
        self.error
    }

    /// The content type part of the "Content-Type" header without
    /// the charset.
    ///
    /// Example:
    ///
    /// ```
    /// # #[cfg(feature = "tls")] {
    /// let resp = ureq::get("https://www.google.com/").call();
    /// assert_eq!("text/html; charset=ISO-8859-1", resp.header("content-type").unwrap());
    /// assert_eq!("text/html", resp.content_type());
    /// # }
    /// ```
    pub fn content_type(&self) -> &str {
        self.header("content-type")
            .map(|header| {
                header
                    .find(';')
                    .map(|index| &header[0..index])
                    .unwrap_or(header)
            })
            .unwrap_or(DEFAULT_CONTENT_TYPE)
    }

    /// The character set part of the "Content-Type" header.native_tls
    ///
    /// Example:
    ///
    /// ```
    /// # #[cfg(feature = "tls")] {
    /// let resp = ureq::get("https://www.google.com/").call();
    /// assert_eq!("text/html; charset=ISO-8859-1", resp.header("content-type").unwrap());
    /// assert_eq!("ISO-8859-1", resp.charset());
    /// # }
    /// ```
    pub fn charset(&self) -> &str {
        charset_from_content_type(self.header("content-type"))
    }

    /// Turn this response into a `impl Read` of the body.
    ///
    /// 1. If `Transfer-Encoding: chunked`, the returned reader will unchunk it
    ///    and any `Content-Length` header is ignored.
    /// 2. If `Content-Length` is set, the returned reader is limited to this byte
    ///    length regardless of how many bytes the server sends.
    /// 3. If no length header, the reader is until server stream end.
    ///
    /// Example:
    ///
    /// ```
    /// # #[cfg(feature = "tls")] {
    /// use std::io::Read;
    ///
    /// let resp =
    ///     ureq::get("https://ureq.s3.eu-central-1.amazonaws.com/hello_world.json")
    ///         .call();
    ///
    /// assert!(resp.has("Content-Length"));
    /// let len = resp.header("Content-Length")
    ///     .and_then(|s| s.parse::<usize>().ok()).unwrap();
    ///
    /// let mut reader = resp.into_reader();
    /// let mut bytes = vec![];
    /// reader.read_to_end(&mut bytes);
    ///
    /// assert_eq!(bytes.len(), len);
    /// # }
    /// ```
    pub fn into_reader(self) -> impl Read + Send {
        //
        let is_http10 = self.http_version().eq_ignore_ascii_case("HTTP/1.0");
        let is_close = self
            .header("connection")
            .map(|c| c.eq_ignore_ascii_case("close"))
            .unwrap_or(false);

        let is_head = (&self.unit).as_ref().map(|u| u.is_head()).unwrap_or(false);
        let has_no_body = is_head
            || match self.status {
                204 | 304 => true,
                _ => false,
            };

        let is_chunked = self
            .header("transfer-encoding")
            .map(|enc| !enc.is_empty()) // whatever it says, do chunked
            .unwrap_or(false);

        let use_chunked = !is_http10 && !has_no_body && is_chunked;

        let limit_bytes = if is_http10 || is_close {
            None
        } else if has_no_body {
            // head requests never have a body
            Some(0)
        } else {
            self.header("content-length")
                .and_then(|l| l.parse::<usize>().ok())
        };

        let stream = self.stream.expect("No reader in response?!");
        let unit = self.unit;
        if let Some(unit) = &unit {
            let result = stream.set_read_timeout(unit.req.timeout_read);
            if let Err(e) = result {
                return Box::new(ErrorReader(e)) as Box<dyn Read + Send>;
            }
        }
        let deadline = unit.as_ref().and_then(|u| u.deadline);
        let stream = DeadlineStream::new(stream, deadline);

        match (use_chunked, limit_bytes) {
            (true, _) => Box::new(PoolReturnRead::new(unit, ChunkDecoder::new(stream))),
            (false, Some(len)) => {
                Box::new(PoolReturnRead::new(unit, LimitedRead::new(stream, len)))
            }
            (false, None) => Box::new(stream),
        }
    }

    /// Turn this response into a String of the response body. By default uses `utf-8`,
    /// but can work with charset, see below.
    ///
    /// This is potentially memory inefficient for large bodies since the
    /// implementation first reads the reader to end into a `Vec<u8>` and then
    /// attempts to decode it using the charset.
    ///
    /// Example:
    ///
    /// ```
    /// # #[cfg(feature = "tls")] {
    /// let resp =
    ///     ureq::get("https://ureq.s3.eu-central-1.amazonaws.com/hello_world.json")
    ///         .call();
    ///
    /// let text = resp.into_string().unwrap();
    ///
    /// assert!(text.contains("hello"));
    /// # }
    /// ```
    ///
    /// ## Charset support
    ///
    /// Requires feature `ureq = { version = "*", features = ["charset"] }`
    ///
    /// Attempts to respect the character encoding of the `Content-Type` header and
    /// falls back to `utf-8`.
    ///
    /// I.e. `Content-Length: text/plain; charset=iso-8859-1` would be decoded in latin-1.
    ///
    pub fn into_string(self) -> io::Result<String> {
        #[cfg(feature = "charset")]
        {
            let encoding = encoding_from_whatwg_label(self.charset())
                .or_else(|| encoding_from_whatwg_label(DEFAULT_CHARACTER_SET))
                .unwrap();
            let mut buf: Vec<u8> = vec![];
            self.into_reader().read_to_end(&mut buf)?;
            Ok(encoding.decode(&buf, DecoderTrap::Replace).unwrap())
        }
        #[cfg(not(feature = "charset"))]
        {
            let mut buf: Vec<u8> = vec![];
            self.into_reader().read_to_end(&mut buf)?;
            Ok(String::from_utf8_lossy(&buf).to_string())
        }
    }

    /// Turn this response into a (serde) JSON value of the response body.
    ///
    /// Requires feature `ureq = { version = "*", features = ["json"] }`
    ///
    /// Example:
    ///
    /// ```
    /// let resp =
    ///     ureq::get("http://ureq.s3.eu-central-1.amazonaws.com/hello_world.json")
    ///         .call();
    ///
    /// let json = resp.into_json().unwrap();
    ///
    /// assert_eq!(json["hello"], "world");
    /// ```
    #[cfg(feature = "json")]
    pub fn into_json(self) -> io::Result<serde_json::Value> {
        use crate::stream::io_err_timeout;
        use std::error::Error;

        let reader = self.into_reader();
        serde_json::from_reader(reader).map_err(|e| {
            // This is to unify TimedOut io::Error in the API.
            // We make a clone of the original error since serde_json::Error doesn't
            // let us get the wrapped error instance back.
            if let Some(ioe) = e.source().and_then(|s| s.downcast_ref::<io::Error>()) {
                if ioe.kind() == ErrorKind::TimedOut {
                    return io_err_timeout(ioe.to_string());
                }
            }

            io::Error::new(
                ErrorKind::InvalidData,
                format!("Failed to read JSON: {}", e),
            )
        })
    }

    /// Turn the body of this response into a type implementing the (serde) Deserialize trait.
    ///
    /// Requires feature `ureq = { version = "*", features = ["json"] }`
    ///
    /// Example:
    ///
    /// ```
    /// # use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct Hello {
    ///     hello: String,
    /// }
    ///
    /// let resp =
    ///     ureq::get("http://ureq.s3.eu-central-1.amazonaws.com/hello_world.json")
    ///         .call();
    ///
    /// let json = resp.into_json_deserialize::<Hello>().unwrap();
    ///
    /// assert_eq!(json.hello, "world");
    /// ```
    #[cfg(feature = "json")]
    pub fn into_json_deserialize<T: DeserializeOwned>(self) -> io::Result<T> {
        let reader = self.into_reader();
        serde_json::from_reader(reader).map_err(|e| {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("Failed to read JSON: {}", e),
            )
        })
    }

    /// Create a response from a Read trait impl.
    ///
    /// This is hopefully useful for unit tests.
    ///
    /// Example:
    ///
    /// ```
    /// use std::io::Cursor;
    ///
    /// let text = "HTTP/1.1 401 Authorization Required\r\n\r\nPlease log in\n";
    /// let read = Cursor::new(text.to_string().into_bytes());
    /// let resp = ureq::Response::from_read(read);
    ///
    /// assert_eq!(resp.status(), 401);
    /// ```
    pub fn from_read(reader: impl Read) -> Self {
        Self::do_from_read(reader).unwrap_or_else(|e| e.into())
    }

    fn do_from_read(mut reader: impl Read) -> Result<Response, Error> {
        //
        // HTTP/1.1 200 OK\r\n
        let status_line = read_next_line(&mut reader)?;

        let (index, status) = parse_status_line(status_line.as_str())?;

        let mut headers: Vec<Header> = Vec::new();
        loop {
            let line = read_next_line(&mut reader)?;
            if line.is_empty() {
                break;
            }
            if let Ok(header) = line.as_str().parse::<Header>() {
                headers.push(header);
            }
        }

        Ok(Response {
            url: None,
            error: None,
            status_line,
            index,
            status,
            headers,
            unit: None,
            stream: None,
        })
    }

    #[cfg(test)]
    pub fn to_write_vec(&self) -> Vec<u8> {
        self.stream.as_ref().unwrap().to_write_vec()
    }
}

/// parse a line like: HTTP/1.1 200 OK\r\n
fn parse_status_line(line: &str) -> Result<(ResponseStatusIndex, u16), Error> {
    //

    let mut split = line.splitn(3, ' ');

    let http_version = split.next().ok_or_else(|| Error::BadStatus)?;
    if http_version.len() < 5 {
        return Err(Error::BadStatus);
    }
    let index1 = http_version.len();

    let status = split.next().ok_or_else(|| Error::BadStatus)?;
    if status.len() < 2 {
        return Err(Error::BadStatus);
    }
    let index2 = index1 + status.len();

    let status = status.parse::<u16>().map_err(|_| Error::BadStatus)?;

    Ok((
        ResponseStatusIndex {
            http_version: index1,
            response_code: index2,
        },
        status,
    ))
}

impl FromStr for Response {
    type Err = Error;
    /// Parse a response from a string.
    ///
    /// Example:
    /// ```
    /// let s = "HTTP/1.1 200 OK\r\n\
    ///     X-Forwarded-For: 1.2.3.4\r\n\
    ///     Content-Type: text/plain\r\n\
    ///     \r\n\
    ///     Hello World!!!";
    /// let resp = s.parse::<ureq::Response>().unwrap();
    /// assert!(resp.has("X-Forwarded-For"));
    /// let body = resp.into_string().unwrap();
    /// assert_eq!(body, "Hello World!!!");
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes().to_owned();
        let mut cursor = Cursor::new(bytes);
        let mut resp = Self::do_from_read(&mut cursor)?;
        set_stream(&mut resp, "".into(), None, Stream::Cursor(cursor));
        Ok(resp)
    }
}

impl Into<Response> for Error {
    fn into(self) -> Response {
        let status = self.status();
        let status_text = self.status_text().to_string();
        let body_text = self.body_text();
        let mut resp = Response::new(status, &status_text, &body_text);
        resp.error = Some(self);
        resp
    }
}

/// "Give away" Unit and Stream to the response.
///
/// *Internal API*
pub(crate) fn set_stream(resp: &mut Response, url: String, unit: Option<Unit>, stream: Stream) {
    resp.url = Some(url);
    resp.unit = unit;
    resp.stream = Some(stream);
}

fn read_next_line<R: Read>(reader: &mut R) -> io::Result<String> {
    let mut buf = Vec::new();
    let mut prev_byte_was_cr = false;
    let mut one = [0_u8];

    loop {
        let amt = reader.read(&mut one[..])?;

        if amt == 0 {
            return Err(io::Error::new(
                ErrorKind::ConnectionAborted,
                "Unexpected EOF",
            ));
        }

        let byte = one[0];

        if byte == b'\n' && prev_byte_was_cr {
            buf.pop(); // removing the '\r'
            return String::from_utf8(buf)
                .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "Header is not in ASCII"));
        }

        prev_byte_was_cr = byte == b'\r';

        buf.push(byte);
    }
}

/// Limits a `Read` to a content size (as set by a "Content-Length" header).
struct LimitedRead<R> {
    reader: R,
    limit: usize,
    position: usize,
}

impl<R: Read> LimitedRead<R> {
    fn new(reader: R, limit: usize) -> Self {
        LimitedRead {
            reader,
            limit,
            position: 0,
        }
    }
}

impl<R: Read> Read for LimitedRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.limit - self.position;
        if left == 0 {
            return Ok(0);
        }
        let from = if left < buf.len() {
            &mut buf[0..left]
        } else {
            buf
        };
        match self.reader.read(from) {
            // https://tools.ietf.org/html/rfc7230#page-33
            // If the sender closes the connection or
            // the recipient times out before the indicated number of octets are
            // received, the recipient MUST consider the message to be
            // incomplete and close the connection.
            Ok(0) => Err(io::Error::new(
                ErrorKind::InvalidData,
                "response body closed before all bytes were read",
            )),
            Ok(amount) => {
                self.position += amount;
                Ok(amount)
            }
            Err(e) => Err(e),
        }
    }
}

#[test]
fn short_read() {
    use std::io::Cursor;
    let mut lr = LimitedRead::new(Cursor::new(vec![b'a'; 3]), 10);
    let mut buf = vec![0; 1000];
    let result = lr.read_to_end(&mut buf);
    assert!(result.is_err());
}

impl<R: Read> From<LimitedRead<R>> for Stream
where
    Stream: From<R>,
{
    fn from(limited_read: LimitedRead<R>) -> Stream {
        limited_read.reader.into()
    }
}

/// Extract the charset from a "Content-Type" header.
///
/// "Content-Type: text/plain; charset=iso8859-1" -> "iso8859-1"
///
/// *Internal API*
pub(crate) fn charset_from_content_type(header: Option<&str>) -> &str {
    header
        .and_then(|header| {
            header.find(';').and_then(|semi| {
                (&header[semi + 1..])
                    .find('=')
                    .map(|equal| (&header[semi + equal + 2..]).trim())
            })
        })
        .unwrap_or(DEFAULT_CHARACTER_SET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_without_charset() {
        let s = "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json\r\n\
                 \r\n\
                 OK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("application/json", resp.content_type());
    }

    #[test]
    fn content_type_with_charset() {
        let s = "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json; charset=iso-8859-4\r\n\
                 \r\n\
                 OK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("application/json", resp.content_type());
    }

    #[test]
    fn content_type_default() {
        let s = "HTTP/1.1 200 OK\r\n\r\nOK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("text/plain", resp.content_type());
    }

    #[test]
    fn charset() {
        let s = "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json; charset=iso-8859-4\r\n\
                 \r\n\
                 OK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("iso-8859-4", resp.charset());
    }

    #[test]
    fn charset_default() {
        let s = "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json\r\n\
                 \r\n\
                 OK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("utf-8", resp.charset());
    }

    #[test]
    fn chunked_transfer() {
        let s = "HTTP/1.1 200 OK\r\n\
                 Transfer-Encoding: Chunked\r\n\
                 \r\n\
                 3\r\n\
                 hel\r\n\
                 b\r\n\
                 lo world!!!\r\n\
                 0\r\n\
                 \r\n";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("hello world!!!", resp.into_string().unwrap());
    }

    #[test]
    #[cfg(feature = "json")]
    fn parse_simple_json() {
        let s = "HTTP/1.1 200 OK\r\n\
             \r\n\
             {\"hello\":\"world\"}";
        let resp = s.parse::<Response>().unwrap();
        let v = resp.into_json().unwrap();
        let compare = "{\"hello\":\"world\"}"
            .parse::<serde_json::Value>()
            .unwrap();
        assert_eq!(v, compare);
    }

    #[test]
    #[cfg(feature = "json")]
    fn parse_deserialize_json() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct Hello {
            hello: String,
        }

        let s = "HTTP/1.1 200 OK\r\n\
             \r\n\
             {\"hello\":\"world\"}";
        let resp = s.parse::<Response>().unwrap();
        let v = resp.into_json_deserialize::<Hello>().unwrap();
        assert_eq!(v.hello, "world");
    }

    #[test]
    fn parse_borked_header() {
        let s = "HTTP/1.1 BORKED\r\n".to_string();
        let resp: Response = s.parse::<Response>().unwrap_err().into();
        assert_eq!(resp.http_version(), "HTTP/1.1");
        assert_eq!(resp.status(), 500);
        assert_eq!(resp.status_text(), "Bad Status");
        assert_eq!(resp.content_type(), "text/plain");
        let v = resp.into_string().unwrap();
        assert_eq!(v, "Bad Status\n");
    }
}

// ErrorReader returns an error for every read.
// The error is as close to a clone of the underlying
// io::Error as we can get.
struct ErrorReader(io::Error);

impl Read for ErrorReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(self.0.kind(), self.0.to_string()))
    }
}
