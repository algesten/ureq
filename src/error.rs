use url::Url;

use std::error;
use std::fmt::{self, Display};
use std::io::{self};

use crate::Response;

/// An error that may occur when processing a [Request](crate::Request).
///
/// This can represent connection-level errors (e.g. connection refused),
/// protocol-level errors (malformed response), or status code errors
/// (e.g. 404 Not Found). For status code errors, [kind()](Error::kind()) will be
/// [ErrorKind::HTTP], [status()](Error::status()) will return the status
/// code, and [into_response()](Error::into_response()) will return the underlying
/// [Response](crate::Response). You can use that Response to, for instance, read
/// the full body (which may contain a useful error message).
///
/// ```
/// use std::{result::Result, time::Duration, thread};
/// use ureq::{Response, Error, Error::Status};
/// # fn main(){ ureq::is_test(true); get_response( "http://httpbin.org/status/500" ); }
///
/// // An example of a function that handles HTTP 429 and 500 errors differently
/// // than other errors. They get retried after a suitable delay, up to 4 times.
/// fn get_response(url: &str) -> Result<Response, Error> {
///     for _ in 1..4 {
///         match ureq::get(url).call() {
///             Err(Status(503, r)) | Err(Status(429, r)) => {
///                 let retry: Option<u64> = r.header("retry-after").and_then(|h| h.parse().ok());
///                 let retry = retry.unwrap_or(5);
///                 eprintln!("{} for {}, retry in {}", r.status(), r.get_url(), retry);
///                 thread::sleep(Duration::from_secs(retry));
///             }
///             result => return result,
///         };
///     }
///     // Ran out of retries; try one last time and return whatever result we get.
///     ureq::get(url).call()
/// }
/// ```
#[derive(Debug)]
pub struct Transport {
    kind: ErrorKind,
    message: Option<String>,
    url: Option<Url>,
    source: Option<Box<dyn error::Error + Send + Sync + 'static>>,
    response: Option<Response>,
}

#[derive(Debug)]
pub enum Error {
    Status(u16, Response),
    Transport(Transport),
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Status(status, response) => {
                write!(f, "{}: status code {}", response.get_url(), status)?;
            }
            Error::Transport(err) => {
                write!(f, "{}", err)?;
            }
        }
        Ok(())
    }
}

impl Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(url) = &self.url {
            write!(f, "{}: ", url)?;
        }
        write!(f, "{}", self.kind)?;
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
        match &self {
            Error::Transport(Transport {
                source: Some(s), ..
            }) => Some(s.as_ref()),
            _ => None,
        }
    }
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: Option<String>) -> Self {
        Error::Transport(Transport {
            kind,
            message,
            url: None,
            source: None,
            response: None,
        })
    }

    pub(crate) fn url(self, url: Url) -> Self {
        if let Error::Transport(mut e) = self {
            e.url = Some(url);
            Error::Transport(e)
        } else {
            self
        }
    }

    pub(crate) fn src(self, e: impl error::Error + Send + Sync + 'static) -> Self {
        if let Error::Transport(mut oe) = self {
            oe.source = Some(Box::new(e));
            Error::Transport(oe)
        } else {
            self
        }
    }

    /// The type of this error.
    ///
    /// ```
    /// # ureq::is_test(true);
    /// let err = ureq::get("http://httpbin.org/status/500")
    ///     .call().unwrap_err();
    /// assert_eq!(err.kind(), ureq::ErrorKind::HTTP);
    /// ```
    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::Status(_, _) => ErrorKind::HTTP,
            Error::Transport(Transport { kind: k, .. }) => k.clone(),
        }
    }

    /// Return true iff the error was due to a connection closing.
    pub(crate) fn connection_closed(&self) -> bool {
        if self.kind() != ErrorKind::Io {
            return false;
        }
        let other_err = match self {
            Error::Status(_, _) => return false,
            Error::Transport(e) => e,
        };
        let source = match other_err.source.as_ref() {
            Some(e) => e,
            None => return false,
        };
        let ioe: &io::Error = match source.downcast_ref() {
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
    InvalidUrl,
    /// The url scheme could not be understood.
    UnknownScheme,
    /// DNS lookup failed.
    Dns,
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
    ProxyUrl,
    /// Proxy could not connect
    ProxyConnect,
    /// Incorrect credentials for proxy
    ProxyUnauthorized,
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

impl From<Response> for Error {
    fn from(resp: Response) -> Error {
        Error::Status(resp.status(), resp)
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        ErrorKind::Io.new().src(err)
    }
}

impl From<Transport> for Error {
    fn from(err: Transport) -> Error {
        Error::Transport(err)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ErrorKind::InvalidUrl => write!(f, "Bad URL"),
            ErrorKind::UnknownScheme => write!(f, "Unknown Scheme"),
            ErrorKind::Dns => write!(f, "Dns Failed"),
            ErrorKind::ConnectionFailed => write!(f, "Connection Failed"),
            ErrorKind::TooManyRedirects => write!(f, "Too Many Redirects"),
            ErrorKind::BadStatus => write!(f, "Bad Status"),
            ErrorKind::BadHeader => write!(f, "Bad Header"),
            ErrorKind::Io => write!(f, "Network Error"),
            ErrorKind::ProxyUrl => write!(f, "Malformed proxy"),
            ErrorKind::ProxyConnect => write!(f, "Proxy failed to connect"),
            ErrorKind::ProxyUnauthorized => write!(f, "Provided proxy credentials are incorrect"),
            ErrorKind::HTTP => write!(f, "HTTP status error"),
        }
    }
}

// #[test]
// fn status_code_error() {
//     let mut err = Error::new(ErrorKind::HTTP, None);
//     err = err.response(Response::new(500, "Internal Server Error", "too much going on").unwrap());
//     assert_eq!(err.to_string(), "status code 500");

//     err = err.url("http://example.com/".parse().unwrap());
//     assert_eq!(err.to_string(), "http://example.com/: status code 500");
// }

#[test]
fn io_error() {
    let ioe = io::Error::new(io::ErrorKind::TimedOut, "too slow");
    let mut err = Error::new(ErrorKind::Io, Some("oops".to_string())).src(ioe);

    err = err.url("http://example.com/".parse().unwrap());
    assert_eq!(
        err.to_string(),
        "http://example.com/: Network Error: oops: too slow"
    );
}

#[test]
fn connection_closed() {
    let ioe = io::Error::new(io::ErrorKind::ConnectionReset, "connection reset");
    let err = ErrorKind::Io.new().src(ioe);
    assert!(err.connection_closed());

    let ioe = io::Error::new(io::ErrorKind::ConnectionAborted, "connection aborted");
    let err = ErrorKind::Io.new().src(ioe);
    assert!(err.connection_closed());
}

#[test]
fn error_is_send_and_sync() {
    fn takes_send(_: impl Send) {}
    fn takes_sync(_: impl Sync) {}
    takes_send(crate::error::ErrorKind::InvalidUrl.new());
    takes_sync(crate::error::ErrorKind::InvalidUrl.new());
}
