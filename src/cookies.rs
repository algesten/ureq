use std::borrow::Cow;
use std::fmt;
use std::iter;
use std::sync::{Mutex, MutexGuard};

use cookie_store::CookieStore;
use http::Uri;

use crate::http;
use crate::util::UriExt;
use crate::Error;

#[cfg(feature = "json")]
use std::io;

#[derive(Debug)]
pub(crate) struct SharedCookieJar {
    inner: Mutex<CookieStore>,
}

/// Collection of cookies.
///
/// The jar is accessed using [`Agent::cookie_jar_lock`][crate::Agent::cookie_jar_lock].
/// It can be saved and loaded.
pub struct CookieJar<'a>(MutexGuard<'a, CookieStore>);

/// Representation of an HTTP cookie.
///
/// Conforms to [IETF RFC6265](https://datatracker.ietf.org/doc/html/rfc6265)
///
/// ## Constructing a `Cookie`
///
/// To construct a cookie it must be parsed and bound to a uri:
///
/// ```
/// use ureq::Cookie;
/// use ureq::http::Uri;
///
/// let uri = Uri::from_static("https://my.server.com");
/// let cookie = Cookie::parse("name=value", &uri)?;
/// assert_eq!(cookie.to_string(), "name=value");
/// # Ok::<_, ureq::Error>(())
/// ```
pub struct Cookie<'a>(CookieInner<'a>);

#[allow(clippy::large_enum_variant)]
enum CookieInner<'a> {
    Borrowed(&'a cookie_store::Cookie<'a>),
    Owned(cookie_store::Cookie<'a>),
}

impl<'a> CookieInner<'a> {
    fn into_static(self) -> cookie_store::Cookie<'static> {
        match self {
            CookieInner::Borrowed(v) => v.clone().into_owned(),
            CookieInner::Owned(v) => v.into_owned(),
        }
    }
}

impl<'a> Cookie<'a> {
    /// Parses a new [`Cookie`] from a string
    pub fn parse<S>(cookie_str: S, uri: &Uri) -> Result<Cookie<'a>, Error>
    where
        S: Into<Cow<'a, str>>,
    {
        let cookie = cookie_store::Cookie::parse(cookie_str, &uri.try_into_url()?)?;
        Ok(Cookie(CookieInner::Owned(cookie)))
    }

    /// The cookie's name.
    pub fn name(&self) -> &str {
        match &self.0 {
            CookieInner::Borrowed(v) => v.name(),
            CookieInner::Owned(v) => v.name(),
        }
    }

    /// The cookie's value.
    pub fn value(&self) -> &str {
        match &self.0 {
            CookieInner::Borrowed(v) => v.value(),
            CookieInner::Owned(v) => v.value(),
        }
    }

    #[cfg(test)]
    fn as_cookie_store(&self) -> &cookie_store::Cookie<'a> {
        match &self.0 {
            CookieInner::Borrowed(v) => v,
            CookieInner::Owned(v) => v,
        }
    }
}

impl Cookie<'static> {
    fn into_owned(self) -> cookie_store::Cookie<'static> {
        match self.0 {
            CookieInner::Owned(v) => v,
            _ => unreachable!(),
        }
    }
}

impl<'a> CookieJar<'a> {
    /// Returns a reference to the __unexpired__ `Cookie` corresponding to the specified `domain`,
    /// `path`, and `name`.
    pub fn get(&self, domain: &str, path: &str, name: &str) -> Option<Cookie<'_>> {
        self.0
            .get(domain, path, name)
            .map(|c| Cookie(CookieInner::Borrowed(c)))
    }

    /// Removes a `Cookie` from the jar, returning the `Cookie` if it was in the jar
    pub fn remove(&mut self, domain: &str, path: &str, name: &str) -> Option<Cookie<'static>> {
        self.0
            .remove(domain, path, name)
            .map(|c| Cookie(CookieInner::Owned(c)))
    }

    /// Inserts `cookie`, received from `uri`, into the jar, following the rules of the
    /// [IETF RFC6265 Storage Model](https://datatracker.ietf.org/doc/html/rfc6265#section-5.3).
    pub fn insert(&mut self, cookie: Cookie<'static>, uri: &Uri) -> Result<(), Error> {
        let url = uri.try_into_url()?;
        self.0.insert(cookie.into_owned(), &url)?;
        Ok(())
    }

    /// Clear the contents of the jar
    pub fn clear(&mut self) {
        self.0.clear()
    }

    /// An iterator visiting all the __unexpired__ cookies in the jar
    pub fn iter(&self) -> impl Iterator<Item = Cookie<'_>> {
        self.0
            .iter_unexpired()
            .map(|c| Cookie(CookieInner::Borrowed(c)))
    }

    /// Serialize any __unexpired__ and __persistent__ cookies in the jar to JSON format and
    /// write them to `writer`
    #[cfg(feature = "json")]
    pub fn save_json<W: io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        Ok(cookie_store::serde::json::save(&self.0, writer)?)
    }

    /// Load JSON-formatted cookies from `reader`, skipping any __expired__ cookies
    ///
    /// Replaces all the contents of the current cookie jar.
    #[cfg(feature = "json")]
    pub fn load_json<R: io::BufRead>(&mut self, reader: R) -> Result<(), Error> {
        let store = cookie_store::serde::json::load(reader)?;
        *self.0 = store;
        Ok(())
    }

    pub(crate) fn store_response_cookies<'b>(
        &mut self,
        iter: impl Iterator<Item = Cookie<'b>>,
        uri: &Uri,
    ) {
        let url = uri.try_into_url().expect("uri to be a url");
        let raw_cookies = iter.map(|c| c.0.into_static().into());
        self.0.store_response_cookies(raw_cookies, &url);
    }

    /// Release the cookie jar.
    pub fn release(self) {}
}

// CookieStore::new() changes parameters depending on feature flag "public_suffix".
// That means if a user enables public_suffix for CookieStore through diamond dependency,
// we start having compilation errors un ureq.
//
// This workaround instantiates a CookieStore in a way that does not change with flags.
fn instantiate_cookie_store() -> CookieStore {
    let i = iter::empty::<Result<cookie_store::Cookie<'static>, &str>>();
    CookieStore::from_cookies(i, true).unwrap()
}

impl SharedCookieJar {
    pub(crate) fn new() -> Self {
        SharedCookieJar {
            inner: Mutex::new(instantiate_cookie_store()),
        }
    }

    pub(crate) fn lock(&self) -> CookieJar<'_> {
        let lock = self.inner.lock().unwrap();
        CookieJar(lock)
    }

    pub(crate) fn get_request_cookies(&self, uri: &Uri) -> String {
        let mut cookies = String::new();

        let url = match uri.try_into_url() {
            Ok(v) => v,
            Err(e) => {
                debug!("Bad url for cookie: {:?}", e);
                return cookies;
            }
        };

        let store = self.inner.lock().unwrap();

        for c in store.matches(&url) {
            if !is_cookie_rfc_compliant(c) {
                debug!("Do not send non compliant cookie: {:?}", c.name());
                continue;
            }

            if !cookies.is_empty() {
                cookies.push(';');
            }

            cookies.push_str(&c.to_string());
        }

        cookies
    }
}

fn is_cookie_rfc_compliant(cookie: &cookie_store::Cookie) -> bool {
    // https://tools.ietf.org/html/rfc6265#page-9
    // set-cookie-header = "Set-Cookie:" SP set-cookie-string
    // set-cookie-string = cookie-pair *( ";" SP cookie-av )
    // cookie-pair       = cookie-name "=" cookie-value
    // cookie-name       = token
    // cookie-value      = *cookie-octet / ( DQUOTE *cookie-octet DQUOTE )
    // cookie-octet      = %x21 / %x23-2B / %x2D-3A / %x3C-5B / %x5D-7E
    //                       ; US-ASCII characters excluding CTLs,
    //                       ; whitespace DQUOTE, comma, semicolon,
    //                       ; and backslash
    // token             = <token, defined in [RFC2616], Section 2.2>

    // https://tools.ietf.org/html/rfc2616#page-17
    // CHAR           = <any US-ASCII character (octets 0 - 127)>
    // ...
    //        CTL            = <any US-ASCII control character
    //                         (octets 0 - 31) and DEL (127)>
    // ...
    //        token          = 1*<any CHAR except CTLs or separators>
    //        separators     = "(" | ")" | "<" | ">" | "@"
    //                       | "," | ";" | ":" | "\" | <">
    //                       | "/" | "[" | "]" | "?" | "="
    //                       | "{" | "}" | SP | HT

    fn is_valid_name(b: &u8) -> bool {
        is_tchar(b)
    }

    fn is_valid_value(b: &u8) -> bool {
        b.is_ascii()
            && !b.is_ascii_control()
            && !b.is_ascii_whitespace()
            && *b != b'"'
            && *b != b','
            && *b != b';'
            && *b != b'\\'
    }

    let name = cookie.name().as_bytes();

    let valid_name = name.iter().all(is_valid_name);

    if !valid_name {
        log::trace!("cookie name is not valid: {:?}", cookie.name());
        return false;
    }

    let value = cookie.value().as_bytes();

    let valid_value = value
        .strip_prefix(br#"""#)
        .and_then(|value| value.strip_suffix(br#"""#))
        .unwrap_or(value)
        .iter()
        .all(is_valid_value);

    if !valid_value {
        // NB. Do not log cookie value since it might be secret
        log::trace!("cookie value is not valid: {:?}", cookie.name());
        return false;
    }

    true
}

#[inline]
pub(crate) fn is_tchar(b: &u8) -> bool {
    match b {
        b'!' | b'#' | b'$' | b'%' | b'&' => true,
        b'\'' | b'*' | b'+' | b'-' | b'.' => true,
        b'^' | b'_' | b'`' | b'|' | b'~' => true,
        b if b.is_ascii_alphanumeric() => true,
        _ => false,
    }
}

impl fmt::Display for Cookie<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            CookieInner::Borrowed(v) => v.fmt(f),
            CookieInner::Owned(v) => v.fmt(f),
        }
    }
}

#[cfg(test)]
mod test {

    use std::convert::TryFrom;

    use super::*;

    fn uri() -> Uri {
        Uri::try_from("https://example.test").unwrap()
    }

    #[test]
    fn illegal_cookie_name() {
        let cookie = Cookie::parse("borked/=value", &uri()).unwrap();
        assert!(!is_cookie_rfc_compliant(cookie.as_cookie_store()));
    }

    #[test]
    fn illegal_cookie_value() {
        let cookie = Cookie::parse("name=borked,", &uri()).unwrap();
        assert!(!is_cookie_rfc_compliant(cookie.as_cookie_store()));
        let cookie = Cookie::parse("name=\"borked", &uri()).unwrap();
        assert!(!is_cookie_rfc_compliant(cookie.as_cookie_store()));
        let cookie = Cookie::parse("name=borked\"", &uri()).unwrap();
        assert!(!is_cookie_rfc_compliant(cookie.as_cookie_store()));
        let cookie = Cookie::parse("name=\"\"borked\"", &uri()).unwrap();
        assert!(!is_cookie_rfc_compliant(cookie.as_cookie_store()));
    }

    #[test]
    fn legal_cookie_name_value() {
        let cookie = Cookie::parse("name=value", &uri()).unwrap();
        assert!(is_cookie_rfc_compliant(cookie.as_cookie_store()));
        let cookie = Cookie::parse("name=\"value\"", &uri()).unwrap();
        assert!(is_cookie_rfc_compliant(cookie.as_cookie_store()));
    }
}
