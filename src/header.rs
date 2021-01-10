use crate::error::{Error, ErrorKind};
use std::fmt;
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

impl fmt::Debug for Header {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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
    pub fn name(&self) -> &str {
        &self.line.as_str()[0..self.index]
    }

    /// The header value.
    pub fn value(&self) -> &str {
        &self.line.as_str()[self.index + 1..].trim()
    }

    /// Compares the given str to the header name ignoring case.
    pub fn is_name(&self, other: &str) -> bool {
        self.name().eq_ignore_ascii_case(other)
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !valid_name(self.name()) || !valid_value(&self.line.as_str()[self.index + 1..]) {
            Err(ErrorKind::BadHeader.msg(&format!("invalid header '{}'", self.line)))
        } else {
            Ok(())
        }
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

// https://tools.ietf.org/html/rfc7230#section-3.2
// Each header field consists of a case-insensitive field name followed
// by a colon (":"), optional leading whitespace, the field value, and
// optional trailing whitespace.
// field-name     = token
// token = 1*tchar
// tchar = "!" / "#" / "$" / "%" / "&" / "'" / "*" / "+" / "-" / "." /
// "^" / "_" / "`" / "|" / "~" / DIGIT / ALPHA
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(is_tchar)
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

// https://tools.ietf.org/html/rfc7230#section-3.2
// Note that field-content has an errata:
// https://www.rfc-editor.org/errata/eid4189
// field-value    = *( field-content / obs-fold )
// field-content  = field-vchar [ 1*( SP / HTAB ) field-vchar ]
// field-vchar    = VCHAR / obs-text
//
// obs-fold       = CRLF 1*( SP / HTAB )
//               ; obsolete line folding
//               ; see Section 3.2.4
// https://tools.ietf.org/html/rfc5234#appendix-B.1
// VCHAR          =  %x21-7E
//                        ; visible (printing) characters
fn valid_value(value: &str) -> bool {
    value.bytes().all(is_field_vchar_or_obs_fold)
}

#[inline]
fn is_field_vchar_or_obs_fold(b: u8) -> bool {
    match b {
        b' ' | b'\t' => true,
        0x21..=0x7E => true,
        _ => false,
    }
}

impl FromStr for Header {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        //
        let line = s.to_string();
        let index = s
            .find(':')
            .ok_or_else(|| ErrorKind::BadHeader.msg("no colon in header"))?;

        // no value?
        if index >= s.len() {
            return Err(ErrorKind::BadHeader.msg("no value in header"));
        }

        let header = Header { line, index };
        header.validate()?;
        Ok(header)
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
fn test_valid_value() {
    assert!(valid_value("example"));
    assert!(valid_value("foo bar"));
    assert!(valid_value(" foobar "));
    assert!(valid_value(" foo\tbar "));
    assert!(valid_value(" foo~"));
    assert!(valid_value(" !bar"));
    assert!(valid_value(" "));
    assert!(!valid_value(" \nfoo"));
    assert!(!valid_value("foo\x7F"));
}

#[test]
fn test_parse_invalid_name() {
    let cases = vec![
        "Content-Type  :",
        " Content-Type: foo",
        "Content-Type foo",
        "\"some-header\": foo",
        "Gödel: Escher, Bach",
        "Foo: \n",
        "Foo: \nbar",
        "Foo: \x7F bar",
    ];
    for c in cases {
        let result = c.parse::<Header>();
        assert!(
            matches!(result, Err(ref e) if e.kind() == ErrorKind::BadHeader),
            "'{}'.parse(): expected BadHeader, got {:?}",
            c,
            result
        );
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

#[test]
fn name_and_value() {
    let header: Header = "X-Forwarded-For: 127.0.0.1".parse().unwrap();
    assert_eq!("X-Forwarded-For", header.name());
    assert_eq!("127.0.0.1", header.value());
    assert!(header.is_name("X-Forwarded-For"));
    assert!(header.is_name("x-forwarded-for"));
    assert!(header.is_name("X-FORWARDED-FOR"));
}
