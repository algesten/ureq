use core::fmt;
use std::io::{self, Read};

use hoot::BodyMode;

use crate::pool::Connection;
use crate::time::Instant;
use crate::unit::Unit;
use crate::Error;

use self::handler::{UnitHandler, UnitHandlerRef};
use self::limit::LimitReader;

mod handler;
mod limit;

#[cfg(feature = "charset")]
mod charset;

#[cfg(feature = "gzip")]
mod gzip;

#[cfg(feature = "brotli")]
mod brotli;

/// A response body returned as [`http::Response<Body>`].
///
/// # Example
///
/// ```
/// use std::io::Read;
/// let mut resp = ureq::get("http://httpbin.org/bytes/100")
///     .call().unwrap();
///
/// assert!(resp.headers().contains_key("Content-Length"));
/// let len: usize = resp.headers().get("Content-Length")
///     .unwrap().to_str().unwrap().parse().unwrap();
///
/// let mut bytes: Vec<u8> = Vec::with_capacity(len);
/// resp.body_mut().as_reader(10_000_000)
///     .read_to_end(&mut bytes).unwrap();
///
/// assert_eq!(bytes.len(), len);
/// ```

pub struct Body {
    info: ResponseInfo,
    unit_handler: UnitHandler,
}

#[derive(Clone)]
pub(crate) struct ResponseInfo {
    content_encoding: ContentEncoding,
    mime_type: Option<String>,
    charset: Option<String>,
    body_mode: BodyMode,
}

impl Body {
    pub(crate) fn new(
        unit: Unit<()>,
        connection: Connection,
        info: ResponseInfo,
        current_time: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        Body {
            info,
            unit_handler: UnitHandler::new(unit, connection, current_time),
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
    /// # Example
    ///
    /// ```
    /// let resp = ureq::get("https://www.google.com/")
    ///     .call().unwrap();
    ///
    /// assert_eq!(resp.body().mime_type(), Some("text/html"));
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
    /// # Example
    ///
    /// ```
    /// let resp = ureq::get("https://www.google.com/")
    ///     .call().unwrap();
    ///
    /// assert_eq!(resp.body().charset(), Some("ISO-8859-1"));
    /// ```
    pub fn charset(&self) -> Option<&str> {
        self.info.charset.as_deref()
    }

    /// Handle this body as a shared `impl Read` of the body.
    ///
    /// # Example
    ///
    /// ```
    /// use std::io::Read;
    ///
    /// let mut resp = ureq::get("http://httpbin.org/bytes/100")
    ///     .call().unwrap();
    ///
    /// let mut bytes: Vec<u8> = Vec::with_capacity(1000);
    /// resp.body_mut().as_reader(1000)
    ///     .read_to_end(&mut bytes).unwrap();
    /// ```
    pub fn as_reader(&mut self, limit: u64) -> BodyReader {
        BodyReader::new(
            LimitReader::new(UnitHandlerRef::Shared(&mut self.unit_handler), limit),
            &self.info,
            // With a shared reader that can be called multiple times, we don't know how
            // much of the incoming body is going to be used. Thus the only reasonable
            // BodyMode for a AsSendBody made from this BodyReader is Chunked.
            BodyMode::Chunked,
        )
    }

    /// Turn this response into an owned `impl Read` of the body.
    ///
    /// ```
    /// use std::io::Read;
    ///
    /// let resp = ureq::get("http://httpbin.org/bytes/100")
    ///     .call().unwrap();
    ///
    /// let (_, body) = resp.into_parts();
    ///
    /// let mut bytes: Vec<u8> = Vec::with_capacity(1000);
    /// body.into_reader(1000)
    ///     .read_to_end(&mut bytes).unwrap();
    /// ```
    pub fn into_reader(self, limit: u64) -> BodyReader<'static> {
        BodyReader::new(
            LimitReader::new(UnitHandlerRef::Owned(self.unit_handler), limit),
            &self.info,
            // Since we are consuming self, we are guaranteed that the reader
            // will read the entire incoming body. Thus if we use the BodyReader
            // for AsSendBody, we can use the incoming body mode to signal outgoing.
            self.info.body_mode,
        )
    }

    /// Read the response as a string.
    ///
    /// Fails if the requested data is not a string.
    ///
    /// ```
    /// let mut resp = ureq::get("http://httpbin.org/robots.txt")
    ///     .call().unwrap();
    ///
    /// let s = resp.body_mut().read_to_string(1000).unwrap();
    /// assert_eq!(s, "User-agent: *\nDisallow: /deny\n");
    /// ```
    pub fn read_to_string(&mut self, limit: usize) -> Result<String, Error> {
        let mut buf = String::new();
        let mut reader = self.as_reader(limit as u64);
        reader.read_to_string(&mut buf)?;
        Ok(buf)
    }

    /// Read the response to a vec.
    ///
    /// ```
    /// let mut resp = ureq::get("http://httpbin.org/bytes/100")
    ///     .call().unwrap();
    ///
    /// let bytes = resp.body_mut().read_to_vec(1000).unwrap();
    /// assert_eq!(bytes.len(), 100);
    /// ```
    pub fn read_to_vec(&mut self, limit: usize) -> Result<Vec<u8>, Error> {
        let mut buf = Vec::new();
        let mut reader = self.as_reader(limit as u64);
        reader.read_to_end(&mut buf)?;
        Ok(buf)
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
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(ContentEncoding::from)
            .unwrap_or(ContentEncoding::None);

        let (mime_type, charset) = headers
            .get("content-type")
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
/// let mut resp = ureq::get("http://httpbin.org/bytes/100")
///     .call().unwrap();
///
/// assert!(resp.headers().contains_key("Content-Length"));
/// let len: usize = resp.headers().get("Content-Length")
///     .unwrap().to_str().unwrap().parse().unwrap();
///
/// let mut bytes: Vec<u8> = Vec::with_capacity(len);
/// resp.body_mut().as_reader(10_000_000)
///     .read_to_end(&mut bytes).unwrap();
///
/// assert_eq!(bytes.len(), len);
/// ```
pub struct BodyReader<'a> {
    reader: CharsetDecoder<ContentDecoder<LimitReader<UnitHandlerRef<'a>>>>,
    body_mode: BodyMode,
}

impl<'a> BodyReader<'a> {
    fn new(
        reader: LimitReader<UnitHandlerRef<'a>>,
        info: &ResponseInfo,
        incoming_body_mode: BodyMode,
    ) -> BodyReader<'a> {
        // This is outgoing body_mode in case we are using the BodyReader as a send body
        // in a proxy situation.
        let mut body_mode = incoming_body_mode;

        let reader = match info.content_encoding {
            ContentEncoding::None | ContentEncoding::Unknown => ContentDecoder::PassThrough(reader),
            #[cfg(feature = "gzip")]
            ContentEncoding::Gzip => {
                body_mode = BodyMode::Chunked;
                ContentDecoder::Gzip(Box::new(gzip::GzipDecoder::new(reader)))
            }
            #[cfg(not(feature = "gzip"))]
            ContentEncoding::Gzip => ContentDecoder::PassThrough(reader),
            #[cfg(feature = "brotli")]
            ContentEncoding::Brotli => {
                body_mode = BodyMode::Chunked;
                ContentDecoder::Brotli(Box::new(brotli::BrotliDecoder::new(reader)))
            }
            #[cfg(not(feature = "brotli"))]
            ContentEncoding::Brotli => ContentDecoder::PassThrough(reader),
        };

        let reader = charset_decoder(
            reader,
            info.mime_type.as_deref(),
            info.charset.as_deref(),
            &mut body_mode,
        );

        BodyReader { body_mode, reader }
    }

    pub(crate) fn body_mode(&self) -> BodyMode {
        self.body_mode
    }
}

#[allow(unused)]
fn charset_decoder<R: Read>(
    reader: R,
    mime_type: Option<&str>,
    charset: Option<&str>,
    body_mode: &mut BodyMode,
) -> CharsetDecoder<R> {
    let is_text = mime_type.map(|m| m.starts_with("text/")).unwrap_or(false);

    if !is_text {
        return CharsetDecoder::PassThrough(reader);
    }

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
            *body_mode = BodyMode::Chunked;
            CharsetDecoder::Decoder(self::charset::CharCodec::new(reader, from, UTF_8))
        }
    }

    #[cfg(not(feature = "charset"))]
    {
        CharsetDecoder::PassThrough(reader)
    }
}

impl<'a> Read for BodyReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

enum CharsetDecoder<R> {
    #[cfg(feature = "charset")]
    Decoder(charset::CharCodec<R>),
    PassThrough(R),
}

impl<R: io::Read> Read for CharsetDecoder<R> {
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

impl<R: Read> Read for ContentDecoder<R> {
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
                info!("Unknown content-encoding: {}", s);
                ContentEncoding::Unknown
            }
        }
    }
}
