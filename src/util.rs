use std::convert::TryFrom;
use std::io::{self, ErrorKind};
use std::{fmt, iter};

use http::header::{ACCEPT, ACCEPT_CHARSET, ACCEPT_ENCODING};
use http::header::{CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE};
use http::header::{DATE, HOST, LOCATION, SERVER, TRANSFER_ENCODING, USER_AGENT};
use http::uri::{Authority, Scheme};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Response, Uri, Version};

use crate::http;
use crate::proxy::ProxyProtocol;
use crate::Error;

pub(crate) mod private {
    pub trait Private {}
}

pub(crate) trait AuthorityExt {
    fn userinfo(&self) -> Option<&str>;
    fn username(&self) -> Option<&str>;
    fn password(&self) -> Option<&str>;
}

// NB: Treating &str with direct indexes is OK, since Uri parsed the Authority,
// and ensured it's all ASCII (or %-encoded).
impl AuthorityExt for Authority {
    fn userinfo(&self) -> Option<&str> {
        let s = self.as_str();
        s.rfind('@').map(|i| &s[..i])
    }

    fn username(&self) -> Option<&str> {
        self.userinfo()
            .map(|a| a.rfind(':').map(|i| &a[..i]).unwrap_or(a))
    }

    fn password(&self) -> Option<&str> {
        self.userinfo()
            .and_then(|a| a.rfind(':').map(|i| &a[i + 1..]))
    }
}

pub(crate) trait SchemeExt {
    fn default_port(&self) -> Option<u16>;
}

impl SchemeExt for Scheme {
    fn default_port(&self) -> Option<u16> {
        if *self == Scheme::HTTPS {
            Some(443)
        } else if *self == Scheme::HTTP {
            Some(80)
        } else if let Ok(proxy) = ProxyProtocol::try_from(self.as_str()) {
            Some(proxy.default_port())
        } else {
            debug!("Unknown scheme: {}", self);
            None
        }
    }
}

/// Windows causes kind `TimedOut` while unix does `WouldBlock`. Since we are not
/// using non-blocking streams, we normalize `WouldBlock` -> `TimedOut`.
pub(crate) trait IoResultExt {
    fn normalize_would_block(self) -> Self;
}

impl<T> IoResultExt for io::Result<T> {
    fn normalize_would_block(self) -> Self {
        match self {
            Ok(v) => Ok(v),
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                Err(io::Error::new(ErrorKind::TimedOut, e))
            }
            Err(e) => Err(e),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ConsumeBuf {
    buf: Vec<u8>,
    filled: usize,
    consumed: usize,
}

impl ConsumeBuf {
    pub fn new(size: usize) -> Self {
        ConsumeBuf {
            buf: vec![0; size],
            filled: 0,
            consumed: 0,
        }
    }

    pub fn resize(&mut self, size: usize) {
        if size > 100 * 1024 * 1024 {
            panic!("ConsumeBuf grown to unreasonable size (>100MB)");
        }
        self.buf.resize(size, 0);
    }

    pub fn add_space(&mut self, size: usize) {
        if size == 0 {
            return;
        }
        let wanted = self.buf.len() + size;
        self.resize(wanted);
    }

    pub fn free_mut(&mut self) -> &mut [u8] {
        self.maybe_shift();
        &mut self.buf[self.filled..]
    }

    pub fn add_filled(&mut self, amount: usize) {
        self.filled += amount;
        assert!(self.filled <= self.buf.len());
    }

    pub fn unconsumed(&self) -> &[u8] {
        &self.buf[self.consumed..self.filled]
    }

    pub fn unconsumed_mut(&mut self) -> &mut [u8] {
        &mut self.buf[self.consumed..self.filled]
    }

    pub fn consume(&mut self, amount: usize) {
        self.consumed += amount;
        assert!(self.consumed <= self.filled);
    }

    fn maybe_shift(&mut self) {
        if self.consumed == 0 {
            return;
        }

        if self.consumed == self.filled {
            self.consumed = 0;
            self.filled = 0;
        } else if self.filled > self.buf.len() / 2 {
            self.buf.copy_within(self.consumed..self.filled, 0);
            self.filled -= self.consumed;
            self.consumed = 0;
        }
    }
}

/// Wrapper to only log non-sensitive data.
pub(crate) struct DebugRequest<'a> {
    pub method: &'a Method,
    pub uri: &'a Uri,
    pub version: Version,
    pub headers: HeaderMap<HeaderValue>,
}

impl<'a> fmt::Debug for DebugRequest<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("uri", &DebugUri(self.uri))
            .field("version", &self.version)
            .field("headers", &DebugHeaders(&self.headers))
            .finish()
    }
}

/// Wrapper to only log non-sensitive data.
pub(crate) struct DebugResponse<'a, B>(pub &'a Response<B>);

impl<'a, B> fmt::Debug for DebugResponse<'a, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.0.status())
            .field("version", &self.0.version())
            .field("headers", &DebugHeaders(self.0.headers()))
            .finish()
    }
}

pub(crate) struct DebugHeaders<'a>(pub &'a HeaderMap);

const NON_SENSITIVE_HEADERS: &[HeaderName] = &[
    DATE,
    CONTENT_TYPE,
    CONTENT_LENGTH,
    TRANSFER_ENCODING,
    CONNECTION,
    CONTENT_ENCODING,
    HOST,
    ACCEPT,
    ACCEPT_ENCODING,
    ACCEPT_CHARSET,
    SERVER,
    USER_AGENT,
    // LOCATION is also logged in a redacted form
];

impl<'a> fmt::Debug for DebugHeaders<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_map();

        static REDACTED_LOCATION: HeaderValue = HeaderValue::from_static("******");
        let has_location = self.0.has_location();

        let filtered_headers = self
            .0
            .iter()
            .filter(|(name, _)| NON_SENSITIVE_HEADERS.contains(name));

        if has_location {
            let location_header = if log_enabled!(log::Level::Trace) {
                iter::once((&LOCATION, self.0.get(LOCATION).unwrap()))
            } else {
                iter::once((&LOCATION, &REDACTED_LOCATION))
            };
            debug.entries(filtered_headers.chain(location_header));
        } else {
            debug.entries(filtered_headers);
        }

        let redact_count = self
            .0
            .iter()
            .filter(|(name, _)| {
                // println!("{}", name);
                !NON_SENSITIVE_HEADERS.contains(name)
            })
            .count()
            // location is logged, but redacted, so do not include in the count
            - if has_location { 1 } else { 0 };

        if redact_count > 0 {
            debug.entry(
                &"<NOTICE>",
                &format!("{} HEADERS ARE REDACTED", redact_count),
            );
        }

        debug.finish()
    }
}

/// Wrapper to only log non-sensitive data.
pub(crate) struct DebugUri<'a>(pub &'a Uri);

impl<'a> fmt::Debug for DebugUri<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = self.0.scheme_str() {
            write!(f, "{}://", s)?;
        }

        if let Some(a) = self.0.authority() {
            write!(f, "{:?}", DebugAuthority(a))?;
        }

        if let Some(q) = self.0.path_and_query() {
            if log_enabled!(log::Level::Trace) {
                write!(f, "{}", q)?;
            } else {
                write!(f, "/******")?;
            }
        }

        Ok(())
    }
}

pub(crate) struct DebugAuthority<'a>(pub &'a Authority);

impl<'a> fmt::Debug for DebugAuthority<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut at = false;

        if let Some(u) = self.0.username() {
            at = true;
            if let Some(x) = u.chars().next() {
                write!(f, "{}*****", x)?;
            }
        }

        if self.0.password().is_some() {
            at = true;
            write!(f, ":******")?;
        }

        if at {
            write!(f, "@")?;
        }

        write!(f, "{}", self.0.host())?;

        if let Some(p) = self.0.port_u16() {
            write!(f, ":{}", p)?;
        }

        Ok(())
    }
}

pub(crate) trait UriExt {
    fn ensure_valid_url(&self) -> Result<(), Error>;

    #[cfg(feature = "_url")]
    fn try_into_url(&self) -> Result<url::Url, Error>;

    #[allow(unused)]
    fn host_port(&self) -> (&str, u16);
}

impl UriExt for Uri {
    fn ensure_valid_url(&self) -> Result<(), Error> {
        let scheme = self
            .scheme()
            .ok_or_else(|| Error::BadUri(format!("{} is missing scheme", self)))?;

        scheme
            .default_port()
            .ok_or_else(|| Error::BadUri(format!("unknown scheme: {}", scheme)))?;

        self.authority()
            .ok_or_else(|| Error::BadUri(format!("{} is missing host", self)))?;

        Ok(())
    }

    #[cfg(feature = "_url")]
    fn try_into_url(&self) -> Result<url::Url, Error> {
        self.ensure_valid_url()?;
        let uri = self.to_string();

        // If ensure_full_url() works, we expect to be able to parse it to a url
        let url = url::Url::parse(&uri).expect("parsed url");

        Ok(url)
    }

    fn host_port(&self) -> (&str, u16) {
        (
            self.host().unwrap(),
            self.port_u16()
                .unwrap_or(self.scheme().unwrap().default_port().unwrap_or(80)),
        )
    }
}

pub(crate) trait HeaderMapExt {
    fn get_str(&self, k: HeaderName) -> Option<&str>;
    fn is_chunked(&self) -> bool;
    fn content_length(&self) -> Option<u64>;
    fn has_accept_encoding(&self) -> bool;
    fn has_user_agent(&self) -> bool;
    fn has_send_body_mode(&self) -> bool {
        self.is_chunked() || self.content_length().is_some()
    }
    fn has_accept(&self) -> bool;
    fn has_content_type(&self) -> bool;
    fn has_location(&self) -> bool;
}

impl HeaderMapExt for HeaderMap {
    fn get_str(&self, k: HeaderName) -> Option<&str> {
        self.get(k).and_then(|v| v.to_str().ok())
    }

    fn is_chunked(&self) -> bool {
        self.get_str(TRANSFER_ENCODING)
            .map(|v| v.contains("chunked"))
            .unwrap_or(false)
    }

    fn content_length(&self) -> Option<u64> {
        let h = self.get_str(CONTENT_LENGTH)?;
        let len: u64 = h.parse().ok()?;
        Some(len)
    }

    fn has_accept_encoding(&self) -> bool {
        self.contains_key(ACCEPT_ENCODING)
    }

    fn has_user_agent(&self) -> bool {
        self.contains_key(USER_AGENT)
    }

    fn has_accept(&self) -> bool {
        self.contains_key(ACCEPT)
    }

    fn has_content_type(&self) -> bool {
        self.contains_key(CONTENT_TYPE)
    }

    fn has_location(&self) -> bool {
        self.contains_key(LOCATION)
    }
}
