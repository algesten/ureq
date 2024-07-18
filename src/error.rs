use std::{fmt, io};

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] http::Error),

    #[error("bad url: {0}")]
    BadUrl(String),

    #[error("{0}")]
    Other(&'static str),

    #[error("protocol: {0}")]
    Protocol(#[from] hoot::Error),

    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("timeout: {0}")]
    Timeout(TimeoutReason),

    #[error("host not found")]
    HostNotFound,

    #[error("redirect failed")]
    RedirectFailed,

    #[error("invalid proxy url")]
    InvalidProxyUrl,

    #[error("connection failed")]
    ConnectionFailed,

    #[error("certificate: {0}")]
    Certificate(&'static str),

    #[error("the response body is larger than request limit")]
    BodyExceedsLimit,

    #[cfg(feature = "rustls")]
    #[error("rustls: {0}")]
    Rustls(#[from] rustls::Error),

    #[cfg(feature = "native-tls")]
    #[error("native-tls: {0}")]
    NativeTls(#[from] native_tls::Error),

    #[cfg(feature = "native-tls")]
    #[error("der: {0}")]
    Der(#[from] der::Error),

    #[cfg(feature = "cookies")]
    #[error("cookie: {0}")]
    Cookie(#[from] cookie_store::CookieError),

    #[cfg(feature = "cookies")]
    #[error("cookie: {0}")]
    CookieJar(#[from] cookie_store::Error),

    #[cfg(feature = "charset")]
    #[error("unknown character set: {0}")]
    UnknownCharset(String),
}

impl Error {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeoutReason {
    Global,
    Resolver,
    OpenConnection,
    SendRequest,
    SendBody,
    Await100,
    RecvResponse,
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn ensure_error_size() {
        // This is platform dependent, so we can't be too strict or precise.
        let size = std::mem::size_of::<Error>();
        println!("Error size: {}", size);
        assert!(size < 100); // 40 on Macbook M1
    }
}
