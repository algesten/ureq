use std::{fmt, io};

use thiserror::Error;

/// Errors from ureq.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// When [`AgentConfig::http_status_as_error`](crate::AgentConfig::http_status_as_error) is true,
    /// 4xx and 5xx response status codes are translated to this error.
    ///
    /// This is the default behavior.
    #[error("http status: {0}")]
    StatusCode(u16),

    /// Errors arising from the http-crate.
    ///
    /// These errors happen for things like invalid characters in header names.
    #[error("http: {0}")]
    Http(#[from] http::Error),

    /// Error if the URI is missing scheme or host.
    #[error("bad uri: {0}")]
    BadUri(String),

    /// An HTTP/1.1 protocol error.
    ///
    /// This can happen if the remote server ends incorrect HTTP data like
    /// missing version or invalid chunked transfer.
    #[error("protocol: {0}")]
    Protocol(#[from] hoot::Error),

    /// Error in io such as the TCP socket.
    #[error("io: {0}")]
    Io(io::Error),

    /// Error raised if the request hits any configured timeout.
    ///
    /// By default no timeouts are set, which means this error can't happen.
    #[error("timeout: {0}")]
    Timeout(TimeoutReason),

    /// Error when resolving a hostname fails.
    #[error("host not found")]
    HostNotFound,

    /// A redirect failed.
    ///
    /// This happens when ureq encounters a redirect when sending a request body
    /// such as a POST request, and receives a 307/308 response. ureq refuses to
    /// redirect the POST body and instead raises this error.
    #[error("redirect failed")]
    RedirectFailed,

    /// Error when creating proxy settings.
    #[error("invalid proxy url")]
    InvalidProxyUrl,

    /// A connection failed.
    #[error("connection failed")]
    ConnectionFailed,

    /// A send body (Such as `&str`) is larger than the `content-length` header.
    #[error("the response body is larger than request limit: {0}")]
    BodyExceedsLimit(u64),

    /// Some error with TLS.
    #[cfg(feature = "_tls")]
    #[error("{0}")]
    Tls(&'static str),

    /// Error in reading PEM certificates/private keys.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "_tls")]
    #[error("PEM: {0:?}")]
    Pem(rustls_pemfile::Error),

    /// An error originating in Rustls.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "rustls")]
    #[error("rustls: {0}")]
    Rustls(#[from] rustls::Error),

    /// An error originating in Native-TLS.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "native-tls")]
    #[error("native-tls: {0}")]
    NativeTls(#[from] native_tls::Error),

    /// An error providing DER encoded certificates or private keys to Native-TLS.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "native-tls")]
    #[error("der: {0}")]
    Der(#[from] der::Error),

    /// An error with the cookies.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "cookies")]
    #[error("cookie: {0}")]
    Cookie(#[from] cookie_store::CookieError),

    /// An error parsing a cookie value.
    #[cfg(feature = "cookies")]
    #[error("{0}")]
    CookieValue(&'static str),

    /// An error in the cookie store.
    ///
    /// *Note:* The wrapped error struct is not considered part of ureq API.
    /// Breaking changes in that struct will not be reflected in ureq
    /// major versions.
    #[cfg(feature = "cookies")]
    #[error("cookie: {0}")]
    CookieJar(#[from] cookie_store::Error),

    /// An unrecognised character set.
    #[cfg(feature = "charset")]
    #[error("unknown character set: {0}")]
    UnknownCharset(String),

    /// The setting [`AgentConfig::https_only`](crate::AgentConfig::https_only) is true and
    /// the URI is not https.
    #[error("agent is configured for https only: {0}")]
    AgentRequireHttpsOnly(String),

    /// The response header, from status up until body, is too big.
    ///
    #[error("response header is too big: {0} > {1}")]
    LargeResponseHeader(usize, usize),

    /// Body decompression failed (gzip or brotli).
    #[error("{0} decompression failed: {1}")]
    #[cfg(any(feature = "gzip", feature = "brotli"))]
    Decompress(&'static str, io::Error),

    /// hoot made no progress and there is no more input to read.
    ///
    /// We should never see this value.
    #[doc(hidden)]
    #[error("body data reading stalled")]
    BodyStalled,
}

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

    pub(crate) fn disconnected() -> Error {
        io::Error::new(io::ErrorKind::UnexpectedEof, "Peer disconnected").into()
    }
}

/// Motivation for an [`Error::Timeout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeoutReason {
    /// Timeout for entire call.
    Global,

    /// Timeout in the resolver.
    Resolver,

    /// Timeout while opening the connection.
    OpenConnection,

    /// Timeout while sending the request headers.
    SendRequest,

    /// Timeout when sending then request body.
    SendBody,

    /// Internal value never seen outside ureq (since awaiting 100 is expected
    /// to timeout).
    #[doc(hidden)]
    Await100,

    /// Timeout while receiving the response headers.
    RecvResponse,

    /// Timeout while receiving the response body.
    RecvBody,
}

impl fmt::Display for TimeoutReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = match self {
            TimeoutReason::Global => "global",
            TimeoutReason::Resolver => "resolver",
            TimeoutReason::OpenConnection => "open connection",
            TimeoutReason::SendRequest => "send request",
            TimeoutReason::SendBody => "send body",
            TimeoutReason::Await100 => "await 100",
            TimeoutReason::RecvResponse => "receive response",
            TimeoutReason::RecvBody => "receive body",
        };
        write!(f, "{}", r)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        let is_wrapped_ureq_error = e.get_ref().map(|x| x.is::<Error>()).unwrap_or(false);

        if is_wrapped_ureq_error {
            // unwraps are ok, see above.
            let boxed = e.into_inner().unwrap();
            let ureq = boxed.downcast::<Error>().unwrap();
            *ureq
        } else {
            Error::Io(e)
        }
    }
}

#[cfg(test)]
mod test {
    use crate::transport::set_handler;

    use super::*;

    #[test]
    fn status_code_error_redirect() {
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
