use std::{fmt, io};

use crate::http;
use crate::Timeout;

/// Errors from ureq.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// When [`http_status_as_error()`](crate::config::ConfigBuilder::http_status_as_error) is true,
    /// 4xx and 5xx response status codes are translated to this error.
    ///
    /// This is the default behavior.
    StatusCode(u16),

    /// Errors arising from the http-crate.
    ///
    /// These errors happen for things like invalid characters in header names.
    Http(http::Error),

    /// Error if the URI is missing scheme or host.
    BadUri(String),

    /// An HTTP/1.1 protocol error.
    ///
    /// This can happen if the remote server ends incorrect HTTP data like
    /// missing version or invalid chunked transfer.
    Protocol(ureq_proto::Error),

    /// Error in io such as the TCP socket.
    Io(io::Error),

    /// Error raised if the request hits any configured timeout.
    ///
    /// By default no timeouts are set, which means this error can't happen.
    Timeout(Timeout),

    /// Error when resolving a hostname fails.
    HostNotFound,

    /// A redirect failed.
    ///
    /// This happens when ureq encounters a redirect when sending a request body
    /// such as a POST request, and receives a 307/308 response. ureq refuses to
    /// redirect the POST body and instead raises this error.
    RedirectFailed,

    /// Error when creating proxy settings.
    InvalidProxyUrl,

    /// A connection failed.
    ///
    /// This is a fallback error when there is no other explanation as to
    /// why a connector chain didn't produce a connection. The idea is that the connector
    /// chain would return some other [`Error`] rather than rely om this value.
    ///
    /// Typically bespoke connector chains should, as far as possible, map their underlying
    /// errors to [`Error::Io`] and use the [`io::ErrorKind`] to provide a reason.
    ///
    /// A bespoke chain is allowed to map to this value, but that provides very little
    /// information to the user as to why the connection failed. One way to mitigate that
    /// would be to rely on the `log` crate to provide additional information.
    ConnectionFailed,

    /// A send body (Such as `&str`) is larger than the `content-length` header.
    BodyExceedsLimit(u64),

    /// Too many redirects.
    ///
    /// The error can be turned off by setting
    /// [`max_redirects_will_error()`](crate::config::ConfigBuilder::max_redirects_will_error)
    /// to false. When turned off, the last response will be returned instead of causing
    /// an error, even if it is a redirect.
    ///
    /// The number of redirects is limited to 10 by default.
    TooManyRedirects,

    /// Some error with TLS.
    // #[cfg(feature = "_tls")] This is deliberately not _tls, because we want to let
    // bespoke TLS transports use this error code.
    Tls(&'static str),

    /// Error in reading PEM certificates/private keys.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "_tls")]
    Pem(rustls_pki_types::pem::Error),

    /// An error originating in Rustls.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "_rustls")]
    Rustls(rustls::Error),

    /// An error originating in Native-TLS.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "native-tls")]
    NativeTls(native_tls::Error),

    /// An error providing DER encoded certificates or private keys to Native-TLS.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "native-tls")]
    Der(der::Error),

    /// An error with the cookies.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "cookies")]
    Cookie(cookie_store::CookieError),

    /// An error parsing a cookie value.
    #[cfg(feature = "cookies")]
    CookieValue(&'static str),

    /// An error in the cookie store.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "cookies")]
    CookieJar(cookie_store::Error),

    /// An unrecognised character set.
    #[cfg(feature = "charset")]
    UnknownCharset(String),

    /// The setting [`https_only`](crate::config::ConfigBuilder::https_only) is true and
    /// the URI is not https.
    RequireHttpsOnly(String),

    /// The response header, from status up until body, is too big.
    LargeResponseHeader(usize, usize),

    /// Body decompression failed (gzip or brotli).
    #[cfg(any(feature = "gzip", feature = "brotli"))]
    Decompress(&'static str, io::Error),

    /// Serde JSON error.
    #[cfg(feature = "json")]
    Json(serde_json::Error),

    /// Invalid MIME type in multipart form.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "multipart")]
    InvalidMimeType(mime_guess::mime::FromStrError),

    /// Attempt to connect to a CONNECT proxy failed.
    ConnectProxyFailed(String),

    /// The protocol requires TLS (https), but the connector did not
    /// create a TLS secured transport.
    ///
    /// This typically indicates a fault in bespoke `Connector` chains.
    TlsRequired,

    /// Some other error occured.
    ///
    /// This is an escape hatch for bespoke connector chains having errors that don't naturally
    /// map to any other error. For connector chains we recommend:
    ///
    /// 1. Map to [`Error::Io`] as far as possible.
    /// 2. Map to other [`Error`] where reasonable.
    /// 3. Fall back on [`Error::Other`].
    /// 4. As a last resort [`Error::ConnectionFailed`].
    ///
    /// ureq does not produce this error using the default connectors.
    Other(Box<dyn std::error::Error + Send + Sync>),

    /// ureq-proto made no progress and there is no more input to read.
    ///
    /// We should never see this value.
    #[doc(hidden)]
    BodyStalled,
}

impl std::error::Error for Error {}

impl Error {
    /// Convert the error into a [`std::io::Error`].
    ///
    /// If the error is [`Error::Io`], we unpack the error. In othe cases we make
    /// an `std::io::ErrorKind::Other`.
    pub fn into_io(self) -> io::Error {
        if let Self::Io(e) = self {
            e
        } else {
            io::Error::new(io::ErrorKind::Other, self)
        }
    }

    pub(crate) fn disconnected(reason: &'static str) -> Error {
        trace!("UnexpectedEof reason: {}", reason);
        io::Error::new(io::ErrorKind::UnexpectedEof, "Peer disconnected").into()
    }
}

pub(crate) fn is_wrapped_ureq_error(e: &io::Error) -> bool {
    e.get_ref().map(|x| x.is::<Error>()).unwrap_or(false)
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if is_wrapped_ureq_error(&e) {
            // unwraps are ok, see above.
            let boxed = e.into_inner().unwrap();
            let ureq = boxed.downcast::<Error>().unwrap();
            *ureq
        } else {
            Error::Io(e)
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::StatusCode(v) => write!(f, "http status: {}", v),
            Error::Http(v) => write!(f, "http: {}", v),
            Error::BadUri(v) => write!(f, "bad uri: {}", v),
            Error::Protocol(v) => write!(f, "protocol: {}", v),
            Error::Io(v) => write!(f, "io: {}", v),
            Error::Timeout(v) => write!(f, "timeout: {}", v),
            Error::HostNotFound => write!(f, "host not found"),
            Error::RedirectFailed => write!(f, "redirect failed"),
            Error::InvalidProxyUrl => write!(f, "invalid proxy url"),
            Error::ConnectionFailed => write!(f, "connection failed"),
            Error::BodyExceedsLimit(v) => {
                write!(f, "the response body is larger than request limit: {}", v)
            }
            Error::TooManyRedirects => write!(f, "too many redirects"),
            Error::Tls(v) => write!(f, "{}", v),
            #[cfg(feature = "_tls")]
            Error::Pem(v) => write!(f, "PEM: {:?}", v),
            #[cfg(feature = "_rustls")]
            Error::Rustls(v) => write!(f, "rustls: {}", v),
            #[cfg(feature = "native-tls")]
            Error::NativeTls(v) => write!(f, "native-tls: {}", v),
            #[cfg(feature = "native-tls")]
            Error::Der(v) => write!(f, "der: {}", v),
            #[cfg(feature = "cookies")]
            Error::Cookie(v) => write!(f, "cookie: {}", v),
            #[cfg(feature = "cookies")]
            Error::CookieValue(v) => write!(f, "{}", v),
            #[cfg(feature = "cookies")]
            Error::CookieJar(v) => write!(f, "cookie: {}", v),
            #[cfg(feature = "charset")]
            Error::UnknownCharset(v) => write!(f, "unknown character set: {}", v),
            Error::RequireHttpsOnly(v) => write!(f, "configured for https only: {}", v),
            Error::LargeResponseHeader(x, y) => {
                write!(f, "response header is too big: {} > {}", x, y)
            }
            #[cfg(any(feature = "gzip", feature = "brotli"))]
            Error::Decompress(x, y) => write!(f, "{} decompression failed: {}", x, y),
            #[cfg(feature = "json")]
            Error::Json(v) => write!(f, "json: {}", v),
            #[cfg(feature = "multipart")]
            Error::InvalidMimeType(v) => write!(f, "invalid MIME type: {}", v),
            Error::ConnectProxyFailed(v) => write!(f, "CONNECT proxy failed: {}", v),
            Error::TlsRequired => write!(f, "TLS required, but transport is unsecured"),
            Error::Other(v) => write!(f, "other: {}", v),
            Error::BodyStalled => write!(f, "body data reading stalled"),
        }
    }
}

impl From<http::Error> for Error {
    fn from(value: http::Error) -> Self {
        Self::Http(value)
    }
}

impl From<ureq_proto::Error> for Error {
    fn from(value: ureq_proto::Error) -> Self {
        Self::Protocol(value)
    }
}

#[cfg(feature = "_rustls")]
impl From<rustls::Error> for Error {
    fn from(value: rustls::Error) -> Self {
        Self::Rustls(value)
    }
}

#[cfg(feature = "native-tls")]
impl From<native_tls::Error> for Error {
    fn from(value: native_tls::Error) -> Self {
        Self::NativeTls(value)
    }
}

#[cfg(feature = "native-tls")]
impl From<der::Error> for Error {
    fn from(value: der::Error) -> Self {
        Self::Der(value)
    }
}

#[cfg(feature = "cookies")]
impl From<cookie_store::CookieError> for Error {
    fn from(value: cookie_store::CookieError) -> Self {
        Self::Cookie(value)
    }
}

#[cfg(feature = "cookies")]
impl From<cookie_store::Error> for Error {
    fn from(value: cookie_store::Error) -> Self {
        Self::CookieJar(value)
    }
}

#[cfg(feature = "json")]
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    #[cfg(feature = "_test")]
    fn status_code_error_redirect() {
        use crate::test::init_test_log;
        use crate::transport::set_handler;
        init_test_log();
        set_handler(
            "/redirect_a",
            302,
            &[("Location", "http://example.edu/redirect_b")],
            &[],
        );
        set_handler(
            "/redirect_b",
            302,
            &[("Location", "http://example.com/status/500")],
            &[],
        );
        set_handler("/status/500", 500, &[], &[]);
        let err = crate::get("http://example.org/redirect_a")
            .call()
            .unwrap_err();
        assert!(matches!(err, Error::StatusCode(500)));
    }

    #[test]
    fn ensure_error_size() {
        // This is platform dependent, so we can't be too strict or precise.
        let size = std::mem::size_of::<Error>();
        assert!(size < 100); // 40 on Macbook M1
    }
}
