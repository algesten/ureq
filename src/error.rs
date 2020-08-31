use crate::response::Response;
use std::fmt;
use std::io::Error as IoError;

#[derive(Debug)]
pub enum Error {
    /// The url could not be understood.
    BadUrl(String),
    /// The url scheme could not be understood.
    UnknownScheme(String),
    /// DNS lookup failed.
    DnsFailed(String),
    /// Connection to server failed.
    ConnectionFailed(String),
    /// Too many redirects.
    TooManyRedirects,
    /// We fail to read the status line. This happens for pooled connections when
    /// TLS fails and we don't notice until trying to read.
    BadStatusRead,
    /// A status line we don't understand `HTTP/1.1 200 OK`.
    BadStatus,
    /// A header line that couldn't be parsed.
    BadHeader,
    /// Some unspecified `std::io::Error`.
    Io(IoError),
    /// Proxy information was not properly formatted
    BadProxy,
    /// Proxy credentials were not properly formatted
    BadProxyCreds,
    /// Proxy could not connect
    ProxyConnect,
    /// Incorrect credentials for proxy
    InvalidProxyCreds,
    /// HTTP status code indicating an error (e.g. 4xx, 5xx)
    /// Read the inner response body for details and to return
    /// the connection to the pool.
    HTTP(Box<Response>),
    /// TLS Error
    #[cfg(feature = "native-tls")]
    TlsError(native_tls::Error),
}

impl Error {
    // If the error is bad status read, which might happen if a TLS connections is
    // closed and we only discover it when trying to read the status line from it.
    pub(crate) fn is_bad_status_read(&self) -> bool {
        match self {
            Error::BadStatusRead => true,
            _ => false,
        }
    }
}

impl From<IoError> for Error {
    fn from(err: IoError) -> Error {
        Error::Io(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for Error {}
