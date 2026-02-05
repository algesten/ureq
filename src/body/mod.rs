use std::fmt;
use std::io;
use std::sync::Arc;

pub use build::BodyBuilder;
use ureq_proto::http::header;
use ureq_proto::BodyMode;

use crate::http;
use crate::run::BodyHandler;
use crate::Error;

use self::limit::LimitReader;
use self::lossy::LossyUtf8Reader;

mod build;
mod limit;
mod lossy;

#[cfg(feature = "charset")]
mod charset;

#[cfg(feature = "gzip")]
mod gzip;

#[cfg(feature = "brotli")]
mod brotli;

/// Default max body size for read_to_string() and read_to_vec().
const MAX_BODY_SIZE: u64 = 10 * 1024 * 1024;

/// A response body returned as [`http::Response<Body>`].
///
/// # Default size limit
///
/// Methods like `read_to_string()`, `read_to_vec()`, and `read_json()` have a **default 10MB limit**
/// to prevent memory exhaustion. To download larger files, use `with_config().limit(new_size)`:
///
/// ```
/// // Download a 20MB file
/// let bytes = ureq::get("http://httpbin.org/bytes/200000000")
///     .call()?
///     .body_mut()
///     .with_config()
///     .limit(20 * 1024 * 1024) // 20MB
///     .read_to_vec()?;
/// # Ok::<_, ureq::Error>(())
/// ```
///
/// # Body lengths
///
/// HTTP/1.1 has two major modes of transfering body data. Either a `Content-Length`
/// header defines exactly how many bytes to transfer, or `Transfer-Encoding: chunked`
/// facilitates a streaming style when the size is not known up front.
///
/// To protect against a problem called [request smuggling], ureq has heuristics for
/// how to interpret a server sending both `Transfer-Encoding` and `Content-Length` headers.
///
/// 1. `chunked` takes precedence if there both headers are present (not for HTTP/1.0)
/// 2. `content-length` is used if there is no chunked
/// 3. If there are no headers, fall back on "close delimited" meaning the socket
///    must close to end the body
///
/// When a `Content-Length` header is used, ureq will ensure the received body is _EXACTLY_
/// as many bytes as declared (it cannot be less). This mechanic is in `ureq-proto`
/// and is different to the [`BodyWithConfig::limit()`] below.
///
/// # Pool reuse
///
/// To return a connection (aka [`Transport`][crate::unversioned::transport::Transport])
/// to the Agent's pool, the body must be read to end. If [`BodyWithConfig::limit()`] is set
/// shorter size than the actual response body, the connection will not be reused.
///
/// # Example
///
/// ```
/// use std::io::Read;
/// let mut res = ureq::get("http://httpbin.org/bytes/100")
///     .call()?;
///
/// assert!(res.headers().contains_key("Content-Length"));
/// let len: usize = res.headers().get("Content-Length")
///     .unwrap().to_str().unwrap().parse().unwrap();
///
/// let mut bytes: Vec<u8> = Vec::with_capacity(len);
/// res.body_mut().as_reader()
///     .read_to_end(&mut bytes)?;
///
/// assert_eq!(bytes.len(), len);
/// # Ok::<_, ureq::Error>(())
/// ```
///
/// [request smuggling]: https://en.wikipedia.org/wiki/HTTP_request_smuggling
pub struct Body {
    source: BodyDataSource,
    info: Arc<ResponseInfo>,
}

enum BodyDataSource {
    Handler(Box<BodyHandler>),
    Reader(Box<dyn io::Read + Send + Sync>),
}

#[derive(Clone)]
pub(crate) struct ResponseInfo {
    content_encoding: ContentEncoding,
    mime_type: Option<String>,
    charset: Option<String>,
    body_mode: BodyMode,
}

impl Body {
    /// Builder for creating a body
    ///
    /// This is useful for testing, or for [`Middleware`][crate::middleware::Middleware] that
    /// returns another body than the requested one.
    pub fn builder() -> BodyBuilder {
        BodyBuilder::new()
    }

    pub(crate) fn new(handler: BodyHandler, info: ResponseInfo) -> Self {
        Body {
            source: BodyDataSource::Handler(Box::new(handler)),
            info: Arc::new(info),
        }
    }

    /// The mime-type of the `content-type` header.
    ///
    /// For the below header, we would get `Some("text/plain")`:
    ///
    /// ```text
    ///     Content-Type: text/plain; charset=iso-8859-1
    /// ```
    ///
    /// *Caution:* A bad server might set `Content-Type` to one thing and send
    /// something else. There is no way ureq can verify this.
    ///
    /// # Example
    ///
    /// ```
    /// let res = ureq::get("https://www.google.com/")
    ///     .call()?;
    ///
    /// assert_eq!(res.body().mime_type(), Some("text/html"));
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn mime_type(&self) -> Option<&str> {
        self.info.mime_type.as_deref()
    }

    /// The charset of the `content-type` header.
    ///
    /// For the below header, we would get `Some("iso-8859-1")`:
    ///
    /// ```text
    ///     Content-Type: text/plain; charset=iso-8859-1
    /// ```
    ///
    /// *Caution:* A bad server might set `Content-Type` to one thing and send
    /// something else. There is no way ureq can verify this.
    ///
    /// # Example
    ///
    /// ```
    /// let res = ureq::get("https://www.google.com/")
    ///     .call()?;
    ///
    /// assert_eq!(res.body().charset(), Some("ISO-8859-1"));
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn charset(&self) -> Option<&str> {
        self.info.charset.as_deref()
    }

    /// The content length of the body.
    ///
    /// This is the value of the `Content-Length` header, if there is one. For chunked
    /// responses (`Transfer-Encoding: chunked`) , this will be `None`. Similarly for
    /// HTTP/1.0 without a `Content-Length` header, the response is close delimited,
    /// which means the length is unknown.
    ///
    /// A bad server might set `Content-Length` to one thing and send something else.
    /// ureq will double check this, see section on body length heuristics.
    ///
    /// # Example
    ///
    /// ```
    /// let res = ureq::get("https://httpbin.org/bytes/100")
    ///     .call()?;
    ///
    /// assert_eq!(res.body().content_length(), Some(100));
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn content_length(&self) -> Option<u64> {
        // After transparent decompression, the original Content-Length no longer
        // reflects the actual body size, so we return None.
        if self.info.is_decompressing() {
            return None;
        }
        match self.info.body_mode {
            BodyMode::NoBody => None,
            BodyMode::LengthDelimited(v) => Some(v),
            BodyMode::Chunked => None,
            BodyMode::CloseDelimited => None,
        }
    }

    /// Handle this body as a shared `impl Read` of the body.
    ///
    /// This is the regular API which goes via [`http::Response::body_mut()`] to get a
    /// mut reference to the `Body`, and then use `as_reader()`. It is also possible to
    /// get a non-shared, owned reader via [`Body::into_reader()`].
    ///
    /// * Reader is not limited by default. That means a malicious server could
    ///   exhaust all avaliable memory on your client machine.
    ///   To set a limit use [`Body::into_with_config()`].
    /// * Reader will error if `Content-Length` is set, but the connection is closed
    ///   before all bytes are received.
    ///
    /// # Example
    ///
    /// ```
    /// use std::io::Read;
    ///
    /// let mut res = ureq::get("http://httpbin.org/bytes/100")
    ///     .call()?;
    ///
    /// let mut bytes: Vec<u8> = Vec::with_capacity(1000);
    /// res.body_mut().as_reader()
    ///     .read_to_end(&mut bytes)?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn as_reader(&mut self) -> BodyReader {
        self.with_config().reader()
    }

    /// Turn this response into an owned `impl Read` of the body.
    ///
    /// Sometimes it might be useful to disconnect the body reader from the body.
    /// The reader returned by [`Body::as_reader()`] borrows the body, while this
    /// variant consumes the body and turns it into a reader with lifetime `'static`.
    /// The reader can for instance be sent to another thread.
    ///
    /// * Reader is not limited by default. That means a malicious server could
    ///   exhaust all avaliable memory on your client machine.
    ///   To set a limit use [`Body::into_with_config()`].
    /// * Reader will error if `Content-Length` is set, but the connection is closed
    ///   before all bytes are received.
    ///
    /// ```
    /// use std::io::Read;
    ///
    /// let res = ureq::get("http://httpbin.org/bytes/100")
    ///     .call()?;
    ///
    /// let (_, body) = res.into_parts();
    ///
    /// let mut bytes: Vec<u8> = Vec::with_capacity(1000);
    /// body.into_reader()
    ///     .read_to_end(&mut bytes)?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn into_reader(self) -> BodyReader<'static> {
        self.into_with_config().reader()
    }

    /// Read the response as a string.
    ///
    /// * Response is limited to 10MB
    /// * Replaces incorrect utf-8 chars to `?`
    ///
    /// To change these defaults use [`Body::with_config()`].
    ///
    /// ```
    /// let mut res = ureq::get("http://httpbin.org/robots.txt")
    ///     .call()?;
    ///
    /// let s = res.body_mut().read_to_string()?;
    /// assert_eq!(s, "User-agent: *\nDisallow: /deny\n");
    /// # Ok::<_, ureq::Error>(())
    /// ```
    ///
    /// For larger text files, you must explicitly increase the limit:
    ///
    /// ```
    /// // Read a large text file (25MB)
    /// let text = ureq::get("http://httpbin.org/get")
    ///     .call()?
    ///     .body_mut()
    ///     .with_config()
    ///     .limit(25 * 1024 * 1024) // 25MB
    ///     .read_to_string()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn read_to_string(&mut self) -> Result<String, Error> {
        self.with_config()
            .limit(MAX_BODY_SIZE)
            .lossy_utf8(true)
            .read_to_string()
    }

    /// Read the response to a vec.
    ///
    /// * Response is limited to 10MB.
    ///
    /// To change this default use [`Body::with_config()`].
    /// ```
    /// let mut res = ureq::get("http://httpbin.org/bytes/100")
    ///     .call()?;
    ///
    /// let bytes = res.body_mut().read_to_vec()?;
    /// assert_eq!(bytes.len(), 100);
    /// # Ok::<_, ureq::Error>(())
    /// ```
    ///
    /// For larger files, you must explicitly increase the limit:
    ///
    /// ```
    /// // Download a larger file (50MB)
    /// let bytes = ureq::get("http://httpbin.org/bytes/200000000")
    ///     .call()?
    ///     .body_mut()
    ///     .with_config()
    ///     .limit(50 * 1024 * 1024) // 50MB
    ///     .read_to_vec()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn read_to_vec(&mut self) -> Result<Vec<u8>, Error> {
        self.with_config()
            //
            .limit(MAX_BODY_SIZE)
            .read_to_vec()
    }

    /// Read the response from JSON.
    ///
    /// * Response is limited to 10MB.
    ///
    /// To change this default use [`Body::with_config()`].
    ///
    /// The returned value is something that derives [`Deserialize`](serde::Deserialize).
    /// You might need to be explicit with which type you want. See example below.
    ///
    /// ```
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct BodyType {
    ///   slideshow: BodyTypeInner,
    /// }
    ///
    /// #[derive(Deserialize)]
    /// struct BodyTypeInner {
    ///   author: String,
    /// }
    ///
    /// let body = ureq::get("https://httpbin.org/json")
    ///     .call()?
    ///     .body_mut()
    ///     .read_json::<BodyType>()?;
    ///
    /// assert_eq!(body.slideshow.author, "Yours Truly");
    /// # Ok::<_, ureq::Error>(())
    /// ```
    ///
    /// For larger JSON files, you must explicitly increase the limit:
    ///
    /// ```
    /// use serde_json::Value;
    ///
    /// // Parse a large JSON file (30MB)
    /// let json: Value = ureq::get("https://httpbin.org/json")
    ///     .call()?
    ///     .body_mut()
    ///     .with_config()
    ///     .limit(30 * 1024 * 1024) // 30MB
    ///     .read_json()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    #[cfg(feature = "json")]
    pub fn read_json<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, Error> {
        let reader = self.with_config().limit(MAX_BODY_SIZE).reader();
        let value: T = serde_json::from_reader(reader)?;
        Ok(value)
    }

    /// Read the body data with configuration.
    ///
    /// This borrows the body which gives easier use with [`http::Response::body_mut()`].
    /// To get a non-borrowed reader use [`Body::into_with_config()`].
    ///
    /// # Example
    ///
    /// ```
    /// let reader = ureq::get("http://httpbin.org/bytes/100")
    ///     .call()?
    ///     .body_mut()
    ///     .with_config()
    ///     // Reader will only read 50 bytes
    ///     .limit(50)
    ///     .reader();
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn with_config(&mut self) -> BodyWithConfig {
        let handler = (&mut self.source).into();
        BodyWithConfig::new(handler, self.info.clone())
    }

    /// Consume self and read the body with configuration.
    ///
    /// This consumes self and returns a reader with `'static` lifetime.
    ///
    /// # Example
    ///
    /// ```
    /// // Get the body out of http::Response
    /// let (_, body) = ureq::get("http://httpbin.org/bytes/100")
    ///     .call()?
    ///     .into_parts();
    ///
    /// let reader = body
    ///     .into_with_config()
    ///     // Reader will only read 50 bytes
    ///     .limit(50)
    ///     .reader();
    /// # Ok::<_, ureq::Error>(())
    /// ```
    ///
    /// This limit behavior can be used to prevent a malicious server from exhausting
    /// memory on the client machine. For example, if the machine running
    /// ureq has 1GB of RAM, you could protect the machine by setting a smaller
    /// limit such as 128MB. The exact number will vary by your client's download
    /// needs, available system resources, and system utilization.
    pub fn into_with_config(self) -> BodyWithConfig<'static> {
        let handler = self.source.into();
        BodyWithConfig::new(handler, self.info)
    }
}

/// Configuration of how to read the body.
///
/// Obtained via one of:
///
/// * [Body::with_config()]
/// * [Body::into_with_config()]
///
/// # Handling large responses
///
/// The `BodyWithConfig` is the primary way to increase the default 10MB size limit
/// when downloading large files to memory:
///
/// ```
/// // Download a 50MB file
/// let large_data = ureq::get("http://httpbin.org/bytes/200000000")
///     .call()?
///     .body_mut()
///     .with_config()
///     .limit(50 * 1024 * 1024) // 50MB
///     .read_to_vec()?;
/// # Ok::<_, ureq::Error>(())
/// ```
pub struct BodyWithConfig<'a> {
    handler: BodySourceRef<'a>,
    info: Arc<ResponseInfo>,
    limit: u64,
    lossy_utf8: bool,
}

impl<'a> BodyWithConfig<'a> {
    fn new(handler: BodySourceRef<'a>, info: Arc<ResponseInfo>) -> Self {
        BodyWithConfig {
            handler,
            info,
            limit: u64::MAX,
            lossy_utf8: false,
        }
    }

    /// Limit the response body.
    ///
    /// Controls how many bytes we should read before throwing an error. This is used
    /// to ensure RAM isn't exhausted by a server sending a very large response body.
    ///
    /// The default limit is `u64::MAX` (unlimited).
    pub fn limit(mut self, value: u64) -> Self {
        self.limit = value;
        self
    }

    /// Replace invalid utf-8 chars.
    ///
    /// `true` means that broken utf-8 characters are replaced by a question mark `?`
    /// (not utf-8 replacement char). This happens after charset conversion regardless of
    /// whether the **charset** feature is enabled or not.
    ///
    /// The default is `false`.
    pub fn lossy_utf8(mut self, value: bool) -> Self {
        self.lossy_utf8 = value;
        self
    }

    fn do_build(self) -> BodyReader<'a> {
        BodyReader::new(
            LimitReader::new(self.handler, self.limit),
            &self.info,
            self.info.body_mode,
            self.lossy_utf8,
        )
    }

    /// Creates a reader.
    ///
    /// The reader is either shared or owned, depending on `with_config` or `into_with_config`.
    ///
    /// # Example of owned vs shared
    ///
    /// ```
    /// // Creates an owned reader.
    /// let reader = ureq::get("https://httpbin.org/get")
    ///     .call()?
    ///     .into_body()
    ///     // takes ownership of Body
    ///     .into_with_config()
    ///     .limit(10)
    ///     .reader();
    /// # Ok::<_, ureq::Error>(())
    /// ```
    ///
    /// ```
    /// // Creates a shared reader.
    /// let reader = ureq::get("https://httpbin.org/get")
    ///     .call()?
    ///     .body_mut()
    ///     // borrows Body
    ///     .with_config()
    ///     .limit(10)
    ///     .reader();
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn reader(self) -> BodyReader<'a> {
        self.do_build()
    }

    /// Read into string.
    ///
    /// *Caution:* without a preceeding [`limit()`][BodyWithConfig::limit], this
    /// becomes an unbounded sized `String`. A bad server could exhaust your memory.
    ///
    /// # Example
    ///
    /// ```
    /// // Reads max 10k to a String.
    /// let string = ureq::get("https://httpbin.org/get")
    ///     .call()?
    ///     .body_mut()
    ///     .with_config()
    ///     // Important. Limits body to 10k
    ///     .limit(10_000)
    ///     .read_to_string()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn read_to_string(self) -> Result<String, Error> {
        use std::io::Read;
        let mut reader = self.do_build();
        let mut buf = String::new();
        reader.read_to_string(&mut buf)?;
        Ok(buf)
    }

    /// Read into vector.
    ///
    /// *Caution:* without a preceeding [`limit()`][BodyWithConfig::limit], this
    /// becomes an unbounded sized `Vec`. A bad server could exhaust your memory.
    ///
    /// # Example
    ///
    /// ```
    /// // Reads max 10k to a Vec.
    /// let myvec = ureq::get("https://httpbin.org/get")
    ///     .call()?
    ///     .body_mut()
    ///     .with_config()
    ///     // Important. Limits body to 10k
    ///     .limit(10_000)
    ///     .read_to_vec()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn read_to_vec(self) -> Result<Vec<u8>, Error> {
        use std::io::Read;
        let mut reader = self.do_build();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Read JSON body.
    ///
    /// *Caution:* without a preceeding [`limit()`][BodyWithConfig::limit], this
    /// becomes an unbounded sized `String`. A bad server could exhaust your memory.
    ///
    /// # Example
    ///
    /// ```
    /// use serde_json::Value;
    ///
    /// // Reads max 10k as a JSON value.
    /// let json: Value  = ureq::get("https://httpbin.org/get")
    ///     .call()?
    ///     .body_mut()
    ///     .with_config()
    ///     // Important. Limits body to 10k
    ///     .limit(10_000)
    ///     .read_json()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    #[cfg(feature = "json")]
    pub fn read_json<T: serde::de::DeserializeOwned>(self) -> Result<T, Error> {
        let reader = self.do_build();
        let value: T = serde_json::from_reader(reader)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy)]
enum ContentEncoding {
    None,
    Gzip,
    Brotli,
    Unknown,
}

impl ResponseInfo {
    pub fn new(headers: &http::HeaderMap, body_mode: BodyMode) -> Self {
        let content_encoding = headers
            .get(header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .map(ContentEncoding::from)
            .unwrap_or(ContentEncoding::None);

        let (mime_type, charset) = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(split_content_type)
            .unwrap_or((None, None));

        ResponseInfo {
            content_encoding,
            mime_type,
            charset,
            body_mode,
        }
    }

    /// Returns true if the body will be decompressed (gzip or brotli).
    pub(crate) fn is_decompressing(&self) -> bool {
        match self.content_encoding {
            #[cfg(feature = "gzip")]
            ContentEncoding::Gzip => true,
            #[cfg(feature = "brotli")]
            ContentEncoding::Brotli => true,
            _ => false,
        }
    }

    /// Whether the mime type indicats text.
    fn is_text(&self) -> bool {
        self.mime_type
            .as_deref()
            .map(|s| s.starts_with("text/"))
            .unwrap_or(false)
    }
}

fn split_content_type(content_type: &str) -> (Option<String>, Option<String>) {
    // Content-Type: text/plain; charset=iso-8859-1
    let mut split = content_type.split(';');

    let Some(mime_type) = split.next() else {
        return (None, None);
    };

    let mut charset = None;

    for maybe_charset in split {
        let maybe_charset = maybe_charset.trim();
        if let Some(s) = maybe_charset.strip_prefix("charset=") {
            charset = Some(s.to_string());
        }
    }

    (Some(mime_type.to_string()), charset)
}

/// A reader of the response data.
///
/// 1. If `Transfer-Encoding: chunked`, the returned reader will unchunk it
///    and any `Content-Length` header is ignored.
/// 2. If `Content-Encoding: gzip` (or `br`) and the corresponding feature
///    flag is enabled (**gzip** and **brotli**), decompresses the body data.
/// 3. Given a header like `Content-Type: text/plain; charset=ISO-8859-1`
///    and the **charset** feature enabled, will translate the body to utf-8.
///    This mechanic need two components a mime-type starting `text/` and
///    a non-utf8 charset indication.
/// 4. If `Content-Length` is set, the returned reader is limited to this byte
///    length regardless of how many bytes the server sends.
/// 5. If no length header, the reader is until server stream end.
/// 6. The limit in the body method used to obtain the reader.
///
/// Note: The reader is also limited by the [`Body::as_reader`] and
/// [`Body::into_reader`] calls. If that limit is set very high, a malicious
/// server might return enough bytes to exhaust available memory. If you're
/// making requests to untrusted servers, you should use set that
/// limit accordingly.
///
/// # Example
///
/// ```
/// use std::io::Read;
/// let mut res = ureq::get("http://httpbin.org/bytes/100")
///     .call()?;
///
/// assert!(res.headers().contains_key("Content-Length"));
/// let len: usize = res.headers().get("Content-Length")
///     .unwrap().to_str().unwrap().parse().unwrap();
///
/// let mut bytes: Vec<u8> = Vec::with_capacity(len);
/// res.body_mut().as_reader()
///     .read_to_end(&mut bytes)?;
///
/// assert_eq!(bytes.len(), len);
/// # Ok::<_, ureq::Error>(())
/// ```
pub struct BodyReader<'a> {
    reader: MaybeLossyDecoder<CharsetDecoder<ContentDecoder<LimitReader<BodySourceRef<'a>>>>>,
    // If this reader is used as SendBody for another request, this
    // body mode can indiciate the content-length. Gzip, charset etc
    // would mean input is not same as output.
    outgoing_body_mode: BodyMode,
}

impl<'a> BodyReader<'a> {
    fn new(
        reader: LimitReader<BodySourceRef<'a>>,
        info: &ResponseInfo,
        incoming_body_mode: BodyMode,
        lossy_utf8: bool,
    ) -> BodyReader<'a> {
        // This is outgoing body_mode in case we are using the BodyReader as a send body
        // in a proxy situation.
        let mut outgoing_body_mode = incoming_body_mode;

        let reader = match info.content_encoding {
            ContentEncoding::None | ContentEncoding::Unknown => ContentDecoder::PassThrough(reader),
            #[cfg(feature = "gzip")]
            ContentEncoding::Gzip => {
                debug!("Decoding gzip");
                outgoing_body_mode = BodyMode::Chunked;
                ContentDecoder::Gzip(Box::new(gzip::GzipDecoder::new(reader)))
            }
            #[cfg(not(feature = "gzip"))]
            ContentEncoding::Gzip => ContentDecoder::PassThrough(reader),
            #[cfg(feature = "brotli")]
            ContentEncoding::Brotli => {
                debug!("Decoding brotli");
                outgoing_body_mode = BodyMode::Chunked;
                ContentDecoder::Brotli(Box::new(brotli::BrotliDecoder::new(reader)))
            }
            #[cfg(not(feature = "brotli"))]
            ContentEncoding::Brotli => ContentDecoder::PassThrough(reader),
        };

        let reader = if info.is_text() {
            charset_decoder(
                reader,
                info.mime_type.as_deref(),
                info.charset.as_deref(),
                &mut outgoing_body_mode,
            )
        } else {
            CharsetDecoder::PassThrough(reader)
        };

        let reader = if info.is_text() && lossy_utf8 {
            MaybeLossyDecoder::Lossy(LossyUtf8Reader::new(reader))
        } else {
            MaybeLossyDecoder::PassThrough(reader)
        };

        BodyReader {
            outgoing_body_mode,
            reader,
        }
    }

    pub(crate) fn body_mode(&self) -> BodyMode {
        self.outgoing_body_mode
    }
}

#[allow(unused)]
fn charset_decoder<R: io::Read>(
    reader: R,
    mime_type: Option<&str>,
    charset: Option<&str>,
    body_mode: &mut BodyMode,
) -> CharsetDecoder<R> {
    #[cfg(feature = "charset")]
    {
        use encoding_rs::{Encoding, UTF_8};

        let from = charset
            .and_then(|c| Encoding::for_label(c.as_bytes()))
            .unwrap_or(UTF_8);

        if from == UTF_8 {
            // Do nothing
            CharsetDecoder::PassThrough(reader)
        } else {
            debug!("Decoding charset {}", from.name());
            *body_mode = BodyMode::Chunked;
            CharsetDecoder::Decoder(self::charset::CharCodec::new(reader, from, UTF_8))
        }
    }

    #[cfg(not(feature = "charset"))]
    {
        CharsetDecoder::PassThrough(reader)
    }
}

enum MaybeLossyDecoder<R> {
    Lossy(LossyUtf8Reader<R>),
    PassThrough(R),
}

impl<R: io::Read> io::Read for MaybeLossyDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            MaybeLossyDecoder::Lossy(r) => r.read(buf),
            MaybeLossyDecoder::PassThrough(r) => r.read(buf),
        }
    }
}

impl<'a> io::Read for BodyReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

enum CharsetDecoder<R> {
    #[cfg(feature = "charset")]
    Decoder(charset::CharCodec<R>),
    PassThrough(R),
}

impl<R: io::Read> io::Read for CharsetDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(feature = "charset")]
            CharsetDecoder::Decoder(v) => v.read(buf),
            CharsetDecoder::PassThrough(v) => v.read(buf),
        }
    }
}

enum ContentDecoder<R: io::Read> {
    #[cfg(feature = "gzip")]
    Gzip(Box<gzip::GzipDecoder<R>>),
    #[cfg(feature = "brotli")]
    Brotli(Box<brotli::BrotliDecoder<R>>),
    PassThrough(R),
}

impl<R: io::Read> io::Read for ContentDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(feature = "gzip")]
            ContentDecoder::Gzip(v) => v.read(buf),
            #[cfg(feature = "brotli")]
            ContentDecoder::Brotli(v) => v.read(buf),
            ContentDecoder::PassThrough(v) => v.read(buf),
        }
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Body").finish()
    }
}

impl From<&str> for ContentEncoding {
    fn from(s: &str) -> Self {
        match s {
            "gzip" => ContentEncoding::Gzip,
            "br" => ContentEncoding::Brotli,
            _ => {
                debug!("Unknown content-encoding: {}", s);
                ContentEncoding::Unknown
            }
        }
    }
}

impl<'a> From<&'a mut BodyDataSource> for BodySourceRef<'a> {
    fn from(value: &'a mut BodyDataSource) -> Self {
        match value {
            BodyDataSource::Handler(v) => Self::HandlerShared(v),
            BodyDataSource::Reader(v) => Self::ReaderShared(v),
        }
    }
}

impl From<BodyDataSource> for BodySourceRef<'static> {
    fn from(value: BodyDataSource) -> Self {
        match value {
            BodyDataSource::Handler(v) => Self::HandlerOwned(v),
            BodyDataSource::Reader(v) => Self::ReaderOwned(v),
        }
    }
}

pub(crate) enum BodySourceRef<'a> {
    HandlerShared(&'a mut BodyHandler),
    HandlerOwned(Box<BodyHandler>),
    ReaderShared(&'a mut (dyn io::Read + Send + Sync)),
    ReaderOwned(Box<dyn io::Read + Send + Sync>),
}

impl<'a> io::Read for BodySourceRef<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            BodySourceRef::HandlerShared(v) => v.read(buf),
            BodySourceRef::HandlerOwned(v) => v.read(buf),
            BodySourceRef::ReaderShared(v) => v.read(buf),
            BodySourceRef::ReaderOwned(v) => v.read(buf),
        }
    }
}

#[cfg(all(test, feature = "_test"))]
mod test {
    use crate::test::init_test_log;
    use crate::transport::set_handler;
    use crate::Error;

    #[test]
    fn content_type_without_charset() {
        init_test_log();
        set_handler("/get", 200, &[("content-type", "application/json")], b"{}");

        let res = crate::get("https://my.test/get").call().unwrap();
        assert_eq!(res.body().mime_type(), Some("application/json"));
        assert!(res.body().charset().is_none());
    }

    #[test]
    fn content_type_with_charset() {
        init_test_log();
        set_handler(
            "/get",
            200,
            &[("content-type", "application/json; charset=iso-8859-4")],
            b"{}",
        );

        let res = crate::get("https://my.test/get").call().unwrap();
        assert_eq!(res.body().mime_type(), Some("application/json"));
        assert_eq!(res.body().charset(), Some("iso-8859-4"));
    }

    #[test]
    fn chunked_transfer() {
        init_test_log();

        let s = "3\r\n\
            hel\r\n\
            b\r\n\
            lo world!!!\r\n\
            0\r\n\
            \r\n";

        set_handler(
            "/get",
            200,
            &[("transfer-encoding", "chunked")],
            s.as_bytes(),
        );

        let mut res = crate::get("https://my.test/get").call().unwrap();
        let b = res.body_mut().read_to_string().unwrap();
        assert_eq!(b, "hello world!!!");
    }

    #[test]
    fn large_response_header() {
        init_test_log();
        set_handler(
            "/get",
            200,
            &[("content-type", &"b".repeat(64 * 1024))],
            b"{}",
        );

        let err = crate::get("https://my.test/get").call().unwrap_err();
        assert!(matches!(err, Error::LargeResponseHeader(_, _)));
    }
}
