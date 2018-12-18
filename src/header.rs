use crate::error::Error;
use ascii::{AsAsciiStr, AsciiString};
use std::str::FromStr;

#[derive(Clone)]
/// Wrapper type for a header line.
pub struct Header {
    line: AsciiString,
    index: usize,
}

impl ::std::fmt::Debug for Header {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(f, "{}", self.line)
    }
}

impl Header {
    pub fn new(name: &str, value: &str) -> Self {
        let s = format!("{}: {}", name, value);
        let line = unsafe { s.as_ascii_str_unchecked().to_owned() };
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
    if !header.name().to_lowercase().starts_with("x-") {
        let name = header.name();
        headers.retain(|h| h.name() != name);
    }
    headers.push(header);
}

impl FromStr for Header {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        //
        let line = AsciiString::from_str(s).map_err(|_| Error::BadHeader)?;
        let index = s.find(':').ok_or_else(|| Error::BadHeader)?;

        // no value?
        if index >= s.len() {
            return Err(Error::BadHeader);
        }

        Ok(Header { line, index })
    }
}
