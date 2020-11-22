use url::Url;

use std::error;
use std::fmt::{self, Display};
use std::io::{self};

use crate::Response;

/// An error that may occur when processing a Request.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: Option<String>,
    url: Option<Url>,
    source: Option<Box<dyn error::Error + Send + Sync + 'static>>,
    response: Option<Box<Response>>,
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(url) = &self.url {
            write!(f, "{}: ", url)?;
        }
        if let Some(response) = &self.response {
            write!(f, "status code {}", response.status())?;
        } else {
            write!(f, "{:?}", self.kind)?;
        }
        if let Some(message) = &self.message {
            write!(f, ": {}", message)?;
        }
        if let Some(source) = &self.source {
            write!(f, ": {}", source)?;
        }
        Ok(())
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.source {
            Some(s) => Some(s.as_ref()),
            None => None,
        }
    }
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: Option<String>) -> Self {
        Error {
            kind,
            message,
            url: None,
            source: None,
            response: None,
        }
    }

    pub(crate) fn url(mut self, url: Url) -> Self {
        self.url = Some(url);
        self
    }

    pub(crate) fn src(mut self, e: impl error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(e));
        self
    }

    pub(crate) fn response(mut self, response: Response) -> Self {
        self.response = Some(Box::new(response));
        self
    }
    pub(crate) fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Return true iff the error was due to a connection closing.
    pub(crate) fn connection_closed(&self) -> bool {
        if self.kind() != ErrorKind::Io {
            return false;
        }
        let source = match self.source.as_ref() {
            Some(e) => e,
            None => return false,
        };
        let ioe: &Box<io::Error> = match source.downcast_ref() {
            Some(e) => e,
            None => return false,
        };
        match ioe.kind() {
            io::ErrorKind::ConnectionAborted => true,
            io::ErrorKind::ConnectionReset => true,
            _ => false,
        }
    }
}

/// One of the types of error the can occur when processing a Request.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ErrorKind {
    /// The url could not be understood.
    BadUrl,
    /// The url scheme could not be understood.
    UnknownScheme,
    /// DNS lookup failed.
    DnsFailed,
    /// Connection to server failed.
    ConnectionFailed,
    /// Too many redirects.
    TooManyRedirects,
    /// A status line we don't understand `HTTP/1.1 200 OK`.
    BadStatus,
    /// A header line that couldn't be parsed.
    BadHeader,
    /// Some unspecified `std::io::Error`.
    Io,
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
    HTTP,
}

impl ErrorKind {
    pub(crate) fn new(self) -> Error {
        Error::new(self, None)
    }

    pub(crate) fn msg(self, s: &str) -> Error {
        Error::new(self, Some(s.to_string()))
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        ErrorKind::Io.new().src(err)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ErrorKind::BadUrl => write!(f, "Bad URL"),
            ErrorKind::UnknownScheme => write!(f, "Unknown Scheme"),
            ErrorKind::DnsFailed => write!(f, "Dns Failed"),
            ErrorKind::ConnectionFailed => write!(f, "Connection Failed"),
            ErrorKind::TooManyRedirects => write!(f, "Too Many Redirects"),
            ErrorKind::BadStatus => write!(f, "Bad Status"),
            ErrorKind::BadHeader => write!(f, "Bad Header"),
            ErrorKind::Io => write!(f, "Network Error"),
            ErrorKind::BadProxy => write!(f, "Malformed proxy"),
            ErrorKind::BadProxyCreds => write!(f, "Failed to parse proxy credentials"),
            ErrorKind::ProxyConnect => write!(f, "Proxy failed to connect"),
            ErrorKind::InvalidProxyCreds => write!(f, "Provided proxy credentials are incorrect"),
            ErrorKind::HTTP => write!(f, "HTTP status error"),
        }
    }
}

#[test]
fn status_code_error() {
    let mut err = Error::new(ErrorKind::HTTP, None);
    err = err.response(Response::new(500, "Internal Server Error", "too much going on").unwrap());
    assert_eq!(err.to_string(), "status code 500");

    err = err.url("http://example.com/".parse().unwrap());
    assert_eq!(err.to_string(), "http://example.com/: status code 500");
}

#[test]
fn io_error() {
    let ioe = io::Error::new(io::ErrorKind::TimedOut, "too slow");
    let mut err = Error::new(ErrorKind::Io, Some("oops".to_string())).src(ioe);

    err = err.url("http://example.com/".parse().unwrap());
    assert_eq!(err.to_string(), "http://example.com/: Io: oops: too slow");
}

#[test]
fn error_is_send_and_sync() {
    fn takes_send(_: impl Send) {}
    fn takes_sync(_: impl Sync) {}
    takes_send(crate::error::ErrorKind::BadUrl.new());
    takes_sync(crate::error::ErrorKind::BadUrl.new());
}
