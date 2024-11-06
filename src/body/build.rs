use std::io::{self, Cursor};
use std::sync::Arc;

use ureq_proto::BodyMode;

use super::{Body, BodyDataSource, ContentEncoding, ResponseInfo};

/// Builder for creating a response body.
///
/// This is useful for testing, or for [`Middleware`][crate::middleware::Middleware] that
/// returns another body than the requested one.
///
/// # Example
///
/// ```
/// use ureq::Body;
/// use ureq::http::Response;
///
/// let body = Body::builder()
///     .mime_type("text/plain")
///     .charset("utf-8")
///     .data("Hello world!");
///
/// let mut response = Response::builder()
///     .status(200)
///     .header("content-type", "text/plain; charset=utf-8")
///     .body(body)?;
///
/// let text = response
///     .body_mut()
///     .read_to_string()?;
///
/// assert_eq!(text, "Hello world!");
/// # Ok::<_, ureq::Error>(())
/// ```
pub struct BodyBuilder {
    info: ResponseInfo,
    limit: Option<u64>,
}

impl BodyBuilder {
    pub(crate) fn new() -> Self {
        BodyBuilder {
            info: ResponseInfo {
                content_encoding: ContentEncoding::None,
                mime_type: None,
                charset: None,
                body_mode: BodyMode::NoBody,
            },
            limit: None,
        }
    }

    /// Set the mime type of the body.
    ///
    /// **This does not set any HTTP headers. Affects Body decoding.**
    ///
    /// ```
    /// use ureq::Body;
    ///
    /// let body = Body::builder()
    ///     .mime_type("text/plain")
    ///     .data("Hello world!");
    /// ```
    pub fn mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.info.mime_type = Some(mime_type.into());
        self
    }

    /// Set the mime type of the body
    ///
    /// **This does not set any HTTP headers. Affects Body decoding.**
    ///
    /// ```
    /// use ureq::Body;
    ///
    /// let body = Body::builder()
    ///     .mime_type("text/plain")
    ///     .charset("utf-8")
    ///     .data("Hello world!");
    /// ```
    pub fn charset(mut self, charset: impl Into<String>) -> Self {
        self.info.charset = Some(charset.into());
        self
    }

    /// Limit how much data is to be released from the body.
    ///
    /// **This does not set any HTTP headers. Affects Body decoding.**
    ///
    /// ```
    /// use ureq::Body;
    ///
    /// let body = Body::builder()
    ///     .mime_type("text/plain")
    ///     .charset("utf-8")
    ///     .limit(5)
    ///     // This will be limited to "Hello"
    ///     .data("Hello world!");
    /// ```
    pub fn limit(mut self, l: u64) -> Self {
        self.limit = Some(l);
        self
    }

    /// Creates the body data turned into bytes.
    ///
    /// Will automatically limit the body reader to the lenth of the data.
    pub fn data(mut self, data: impl Into<Vec<u8>>) -> Body {
        let data: Vec<u8> = data.into();

        let len = self.limit.unwrap_or(data.len() as u64);
        self.info.body_mode = BodyMode::LengthDelimited(len);

        self.reader(Cursor::new(data))
    }

    /// Creates a body from a streaming reader.
    ///
    /// The reader can be limited by using `.limit()` or that the reader
    /// reaches the end.
    pub fn reader(self, data: impl io::Read + Send + Sync + 'static) -> Body {
        Body {
            source: BodyDataSource::Reader(Box::new(data)),
            info: Arc::new(self.info),
        }
    }
}
