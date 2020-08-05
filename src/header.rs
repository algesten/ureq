use crate::error::Error;
use std::str::FromStr;

#[derive(Clone, PartialEq)]
/// Wrapper type for a header field.
/// https://tools.ietf.org/html/rfc7230#section-3.2
pub struct Header {
    // Line contains the unmodified bytes of single header field.
    // It does not contain the final CRLF.
    line: String,
    // Index is the position of the colon within the header field.
    // Invariant: index > 0
    // Invariant: index + 1 < line.len()
    index: usize,
}

impl ::std::fmt::Debug for Header {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(f, "{}", self.line)
    }
}

impl Header {
    pub fn new(name: &str, value: &str) -> Self {
        let line = format!("{}: {}", name, value);
        let index = name.len();
        Header { line, index }
    }

    /// The header name.
    ///
    /// ```
    /// let header = "X-Forwarded-For: 127.0.0.1"
    ///     .parse::<ureq::Header>()
    ///     .unwrap();
    /// assert_eq!("X-Forwarded-For", header.name());
    /// ```
    pub fn name(&self) -> &str {
        &self.line.as_str()[0..self.index]
    }

    /// The header value.
    ///
    /// ```
    /// let header = "X-Forwarded-For: 127.0.0.1"
    ///     .parse::<ureq::Header>()
    ///     .unwrap();
    /// assert_eq!("127.0.0.1", header.value());
    /// ```
    pub fn value(&self) -> &str {
        &self.line.as_str()[self.index + 1..].trim()
    }

    /// Compares the given str to the header name ignoring case.
    ///
    /// ```
    /// let header = "X-Forwarded-For: 127.0.0.1"
    ///     .parse::<ureq::Header>()
    ///     .unwrap();
    /// assert!(header.is_name("x-forwarded-for"));
    /// ```
    pub fn is_name(&self, other: &str) -> bool {
        self.name().eq_ignore_ascii_case(other)
    }
}

pub fn get_header<'a, 'b>(headers: &'b [Header], name: &'a str) -> Option<&'b str> {
    headers.iter().find(|h| h.is_name(name)).map(|h| h.value())
}

pub fn get_all_headers<'a, 'b>(headers: &'b [Header], name: &'a str) -> Vec<&'b str> {
    headers
        .iter()
        .filter(|h| h.is_name(name))
        .map(|h| h.value())
        .collect()
}

pub fn has_header(headers: &[Header], name: &str) -> bool {
    get_header(headers, name).is_some()
}

pub fn add_header(headers: &mut Vec<Header>, header: Header) {
    let name = header.name();
    if !name.starts_with("x-") && !name.starts_with("X-") {
        headers.retain(|h| h.name() != name);
    }
    headers.push(header);
}

// https://tools.ietf.org/html/rfc7230#section-3.2.3
// Each header field consists of a case-insensitive field name followed
// by a colon (":"), optional leading whitespace, the field value, and
// optional trailing whitespace.
// field-name     = token
// token = 1*tchar
// tchar = "!" / "#" / "$" / "%" / "&" / "'" / "*" / "+" / "-" / "." /
// "^" / "_" / "`" / "|" / "~" / DIGIT / ALPHA
fn valid_name(name: &str) -> bool {
    name.len() > 0 && name.bytes().all(is_tchar)
}

#[inline]
fn is_tchar(b: u8) -> bool {
    match b {
        b'!' | b'#' | b'$' | b'%' | b'&' => true,
        b'\'' | b'*' | b'+' | b'-' | b'.' => true,
        b'^' | b'_' | b'`' | b'|' | b'~' => true,
        b if b.is_ascii_alphanumeric() => true,
        _ => false,
    }
}

impl FromStr for Header {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        //
        let line = s.to_string();
        let index = s.find(':').ok_or_else(|| Error::BadHeader)?;

        // no value?
        if index >= s.len() {
            return Err(Error::BadHeader);
        }

        if !valid_name(&line[0..index]) {
            return Err(Error::BadHeader);
        }

        Ok(Header { line, index })
    }
}

#[test]
fn test_valid_name() {
    assert!(valid_name("example"));
    assert!(valid_name("Content-Type"));
    assert!(valid_name("h-123456789"));
    assert!(!valid_name("Content-Type:"));
    assert!(!valid_name("Content-Type "));
    assert!(!valid_name(" some-header"));
    assert!(!valid_name("\"invalid\""));
    assert!(!valid_name("Gödel"));
}

#[test]
fn test_parse_invalid_name() {
    let h = "Content-Type   :".parse::<Header>();
    match h {
        Err(Error::BadHeader) => {}
        h => assert!(false, "expected BadHeader error, got {:?}", h),
    }
}

#[test]
fn empty_value() {
    let h = "foo:".parse::<Header>().unwrap();
    assert_eq!(h.value(), "");
}

#[test]
fn value_with_whitespace() {
    let h = "foo:      bar    ".parse::<Header>().unwrap();
    assert_eq!(h.value(), "bar");
}
