use std::io::Error as IoError;

#[cfg(feature = "tls")]
use std::net::TcpStream;
#[cfg(feature = "tls")]
use native_tls::Error as TlsError;
#[cfg(feature = "tls")]
use native_tls::HandshakeError;

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
    Io(IoError),
    /// Some unspecified TLS error. Synthetic error `400`.
    #[cfg(feature = "tls")]
    Tls(TlsError),
    /// Some unspecified TLS handshake error. Synthetic error `500`.
    #[cfg(feature = "tls")]
    TlsHandshake(HandshakeError<TcpStream>),
}

impl Error {
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
            #[cfg(feature = "tls")]
            Error::Tls(_) => 400,
            #[cfg(feature = "tls")]
            Error::TlsHandshake(_) => 500,
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
            #[cfg(feature = "tls")]
            Error::Tls(_) => "TLS Error",
            #[cfg(feature = "tls")]
            Error::TlsHandshake(_) => "TLS Handshake Error",
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
            #[cfg(feature = "tls")]
            Error::Tls(tls) => format!("TLS Error: {}", tls),
            #[cfg(feature = "tls")]
            Error::TlsHandshake(he) => format!("TLS Handshake Error: {}", he),
        }
    }
}

impl From<IoError> for Error {
    fn from(err: IoError) -> Error {
        Error::Io(err)
    }
}

#[cfg(feature = "tls")]
impl From<TlsError> for Error {
    fn from(err: TlsError) -> Error {
        Error::Tls(err)
    }
}

#[cfg(feature = "tls")]
impl From<HandshakeError<TcpStream>> for Error {
    fn from(err: HandshakeError<TcpStream>) -> Error {
        Error::TlsHandshake(err)
    }
}
