use crate::response::Response;
use std::fmt;
use std::io::{self, ErrorKind};

/// Errors that may be emitted when performing a [Request][crate::Request].
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
    /// A status line we don't understand `HTTP/1.1 200 OK`.
    BadStatus,
    /// A header line that couldn't be parsed.
    BadHeader,
    /// Some unspecified `std::io::Error`.
    Io(io::Error),
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
}

impl Error {
    // Return true iff the error was due to a connection closing.
    pub(crate) fn connection_closed(&self) -> bool {
        match self {
            Error::Io(e) if e.kind() == ErrorKind::ConnectionAborted => true,
            Error::Io(e) if e.kind() == ErrorKind::ConnectionReset => true,
            _ => false,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::Io(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::BadUrl(url) => write!(f, "Bad URL: {}", url),
            Error::UnknownScheme(scheme) => write!(f, "Unknown Scheme: {}", scheme),
            Error::DnsFailed(err) => write!(f, "Dns Failed: {}", err),
            Error::ConnectionFailed(err) => write!(f, "Connection Failed: {}", err),
            Error::TooManyRedirects => write!(f, "Too Many Redirects"),
            Error::BadStatus => write!(f, "Bad Status"),
            Error::BadHeader => write!(f, "Bad Header"),
            Error::Io(ioe) => write!(f, "Network Error: {}", ioe),
            Error::BadProxy => write!(f, "Malformed proxy"),
            Error::BadProxyCreds => write!(f, "Failed to parse proxy credentials"),
            Error::ProxyConnect => write!(f, "Proxy failed to connect"),
            Error::InvalidProxyCreds => write!(f, "Provided proxy credentials are incorrect"),
            Error::HTTP(response) => write!(f, "HTTP status {}", response.status()),
        }
    }
}

impl std::error::Error for Error {}
