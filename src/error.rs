use std::io::Error as IoError;
use native_tls::Error as TlsError;
use native_tls::HandshakeError;
use std::net::TcpStream;

#[derive(Debug)]
pub enum Error {
    BadUrl(String),
    UnknownScheme(String),
    DnsFailed(String),
    ConnectionFailed(String),
    TooManyRedirects,
    BadStatus,
    BadHeader,
    Io(IoError),
    Tls(TlsError),
    TlsHandshake(HandshakeError<TcpStream>),
}

impl Error {
    pub fn status(&self) -> u16 {
        match self {
            Error::BadUrl(_) => 400,
            Error::UnknownScheme(_) => 400,
            Error::DnsFailed(_) => 400,
            Error::ConnectionFailed(_) => 500,
            Error::TooManyRedirects => 400,
            Error::BadStatus => 500,
            Error::BadHeader => 500,
            Error::Io(_) => 500,
            Error::Tls(_) => 400,
            Error::TlsHandshake(_) => 500,
        }
    }
    pub fn status_text(&self) -> &str {
        match self {
            Error::BadUrl(e) => {
                println!("{}", e);
                "Bad URL"
            },
            Error::UnknownScheme(_) => "Unknown Scheme",
            Error::DnsFailed(_) => "Dns Failed",
            Error::ConnectionFailed(_) => "Connection Failed",
            Error::TooManyRedirects => "Too Many Redirects",
            Error::BadStatus => "Bad Status",
            Error::BadHeader => "Bad Header",
            Error::Io(_) => "Network Error",
            Error::Tls(_) => "TLS Error",
            Error::TlsHandshake(_) => "TLS Handshake Error",
        }
    }
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
            Error::Tls(tls) => format!("TLS Error: {}", tls),
            Error::TlsHandshake(he) => format!("TLS Handshake Error: {}", he),
        }
    }
}

impl From<IoError> for Error {
    fn from(err: IoError) -> Error {
        Error::Io(err)
    }
}

impl From<TlsError> for Error {
    fn from(err: TlsError) -> Error {
        Error::Tls(err)
    }
}

impl From<HandshakeError<TcpStream>> for Error {
    fn from(err: HandshakeError<TcpStream>) -> Error {
        Error::TlsHandshake(err)
    }
}
