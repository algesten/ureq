use std::{fmt, io};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
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

#[derive(Debug)]
pub enum TimeoutReason {
    Resolver,
    Global,
    Call,
    Socks,
}

impl fmt::Display for TimeoutReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeoutReason::Resolver => write!(f, "resolver"),
            TimeoutReason::Global => write!(f, "global"),
            TimeoutReason::Call => write!(f, "call"),
            TimeoutReason::Socks => write!(f, "socks"),
        }
    }
}
