use core::fmt;
use std::convert::TryFrom;
use std::hash::{Hash, Hasher};
use std::io::{self, ErrorKind};
use std::ops::{Deref, DerefMut};

use http::uri::{Authority, Scheme};
use http::{HeaderMap, Request, Response, Uri};

use crate::proxy::Proto;
use crate::Error;

pub struct Secret<T>(T);

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(******)")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "******")
    }
}

impl<T> Deref for Secret<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Secret<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Clone> Clone for Secret<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: PartialEq> PartialEq for Secret<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Eq> Eq for Secret<T> {}

impl<T: Hash> Hash for Secret<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T: Default> Default for Secret<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

pub trait AuthorityExt {
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

pub trait SchemeExt {
    fn default_port(&self) -> u16;
}

impl SchemeExt for Scheme {
    fn default_port(&self) -> u16 {
        if *self == Scheme::HTTPS {
            443
        } else if *self == Scheme::HTTP {
            80
        } else if let Ok(proxy) = Proto::try_from(self.as_str()) {
            proxy.default_port()
        } else {
            panic!("Unknown scheme: {}", self);
        }
    }
}

/// Windows causes kind `TimedOut` while unix does `WouldBlock`. Since we are not
/// using non-blocking streams, we normalize `WouldBlock` -> `TimedOut`.
pub trait IoResultExt {
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

/// Wrapper to only log non-sensitive data.
pub struct DebugRequest<'a, B>(pub &'a Request<B>);

impl<'a, B> fmt::Debug for DebugRequest<'a, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.0.method())
            .field("version", &self.0.version())
            .field("headers", &DebugHeaders(self.0.headers()))
            .finish()
    }
}

/// Wrapper to only log non-sensitive data.
pub struct DebugResponse<'a, B>(pub &'a Response<B>);

impl<'a, B> fmt::Debug for DebugResponse<'a, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.0.status())
            .field("version", &self.0.version())
            .field("headers", &DebugHeaders(self.0.headers()))
            .finish()
    }
}

struct DebugHeaders<'a>(&'a HeaderMap);

const NON_SENSITIVE_HEADERS: &[&str] = &[
    "date",
    "content-type",
    "content-length",
    "transfer-encoding",
    "connection",
    "location",
    "content-encoding",
];

impl<'a> fmt::Debug for DebugHeaders<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_map();
        debug.entries(
            self.0
                .iter()
                .filter(|(name, _)| NON_SENSITIVE_HEADERS.contains(&name.as_str())),
        );

        let redact_count = self
            .0
            .iter()
            .filter(|(name, _)| !NON_SENSITIVE_HEADERS.contains(&name.as_str()))
            .count();

        if redact_count > 0 {
            debug.entry(
                &"<NOTICE>",
                &format!("{} HEADERS ARE REDACTED", redact_count),
            );
        }

        debug.finish()
    }
}

pub trait UriExt {
    fn ensure_full_url(&self) -> Result<(), Error>;

    #[cfg(feature = "_url")]
    fn try_into_url(&self) -> Result<url::Url, Error>;
}

impl UriExt for Uri {
    fn ensure_full_url(&self) -> Result<(), Error> {
        self.scheme()
            .ok_or_else(|| Error::BadUrl(format!("{} is missing scheme", self)))?;

        self.authority()
            .ok_or_else(|| Error::BadUrl(format!("{} is missing host/port", self)))?;

        Ok(())
    }

    #[cfg(feature = "_url")]
    fn try_into_url(&self) -> Result<url::Url, Error> {
        self.ensure_full_url()?;
        let uri = self.to_string();

        // If ensure_full_url() works, we expect to be able to parse it to a url
        let url = url::Url::parse(&uri).expect("parsed url");

        Ok(url)
    }
}

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
        self.buf.resize(size, 0);
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
        } else if self.filled > self.buf.len() {
            self.buf.copy_within(self.consumed..self.filled, 0);
            self.filled -= self.consumed;
            self.consumed = 0;
        }
    }
}

impl Deref for ConsumeBuf {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.unconsumed()
    }
}
