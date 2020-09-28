use std::fmt;
use std::io::{self, ErrorKind};

/// Errors that are translated to ["synthetic" responses](struct.Response.html#method.synthetic).
#[derive(Debug)]
pub enum Error {
    /// The url could not be understood. Synthetic error `400`.
    BadUrl(String),
    /// The url scheme could not be understood. Synthetic error `400`.
    UnknownScheme(String),
    /// DNS lookup failed. Synthetic error `400`.
    DnsFailed(String),
    /// Connection to server failed. Synthetic error `500`.
    ConnectionFailed(String),
    /// Too many redirects. Synthetic error `500`.
    TooManyRedirects,
    /// A status line we don't understand `HTTP/1.1 200 OK`. Synthetic error `500`.
    BadStatus,
    /// A header line that couldn't be parsed. Synthetic error `500`.
    BadHeader,
    /// Some unspecified `std::io::Error`. Synthetic error `500`.
    Io(io::Error),
    /// Proxy information was not properly formatted
    BadProxy,
    /// Proxy credentials were not properly formatted
    BadProxyCreds,
    /// Proxy could not connect
    ProxyConnect,
    /// Incorrect credentials for proxy
    InvalidProxyCreds,
    /// TLS Error
    #[cfg(feature = "native-tls")]
    TlsError(native_tls::Error),
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

    /// For synthetic responses, this is the error code.
    pub fn status(&self) -> u16 {
        match self {
            Error::BadUrl(_) => 400,
            Error::UnknownScheme(_) => 400,
            Error::DnsFailed(_) => 400,
            Error::ConnectionFailed(_) => 500,
            Error::TooManyRedirects => 500,
            Error::BadStatus => 500,
            Error::BadHeader => 500,
            Error::Io(_) => 500,
            Error::BadProxy => 500,
            Error::BadProxyCreds => 500,
            Error::ProxyConnect => 500,
            Error::InvalidProxyCreds => 500,
            #[cfg(feature = "native-tls")]
            Error::TlsError(_) => 500,
        }
    }

    /// For synthetic responses, this is the status text.
    pub fn status_text(&self) -> &str {
        match self {
            Error::BadUrl(_) => "Bad URL",
            Error::UnknownScheme(_) => "Unknown Scheme",
            Error::DnsFailed(_) => "Dns Failed",
            Error::ConnectionFailed(_) => "Connection Failed",
            Error::TooManyRedirects => "Too Many Redirects",
            Error::BadStatus => "Bad Status",
            Error::BadHeader => "Bad Header",
            Error::Io(_) => "Network Error",
            Error::BadProxy => "Malformed proxy",
            Error::BadProxyCreds => "Failed to parse proxy credentials",
            Error::ProxyConnect => "Proxy failed to connect",
            Error::InvalidProxyCreds => "Provided proxy credentials are incorrect",
            #[cfg(feature = "native-tls")]
            Error::TlsError(_) => "TLS Error",
        }
    }

    /// For synthetic responses, this is the body text.
    pub fn body_text(&self) -> String {
        match self {
            Error::BadUrl(url) => format!("Bad URL: {}", url),
            Error::UnknownScheme(scheme) => format!("Unknown Scheme: {}", scheme),
            Error::DnsFailed(err) => format!("Dns Failed: {}", err),
            Error::ConnectionFailed(err) => format!("Connection Failed: {}", err),
            Error::TooManyRedirects => "Too Many Redirects".to_string(),
            Error::BadStatus => "Bad Status".to_string(),
            Error::BadHeader => "Bad Header".to_string(),
            Error::Io(ioe) => format!("Network Error: {}", ioe),
            Error::BadProxy => "Malformed proxy".to_string(),
            Error::BadProxyCreds => "Failed to parse proxy credentials".to_string(),
            Error::ProxyConnect => "Proxy failed to connect".to_string(),
            Error::InvalidProxyCreds => "Provided proxy credentials are incorrect".to_string(),
            #[cfg(feature = "native-tls")]
            Error::TlsError(err) => format!("TLS Error: {}", err),
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
        write!(f, "{}", self.body_text())
    }
}

impl std::error::Error for Error {}
