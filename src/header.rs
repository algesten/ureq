use ascii::AsciiString;
use error::Error;
use std::str::FromStr;

#[derive(Debug, Clone)]
/// Wrapper type for a header line.
pub struct Header {
    line: AsciiString,
    index: usize,
}

impl Header {
    /// The header name.
    ///
    /// ```
    /// let header = "X-Forwarded-For: 127.0.0.1".parse::<ureq::Header>().unwrap();
    /// assert_eq!("X-Forwarded-For", header.name());
    /// ```
    pub fn name(&self) -> &str {
        &self.line.as_str()[0..self.index]
    }

    /// The header value.
    ///
    /// ```
    /// let header = "X-Forwarded-For: 127.0.0.1".parse::<ureq::Header>().unwrap();
    /// assert_eq!("127.0.0.1", header.value());
    /// ```
    pub fn value(&self) -> &str {
        &self.line.as_str()[self.index + 1..].trim()
    }

    /// Compares the given str to the header name ignoring case.
    ///
    /// ```
    /// let header = "X-Forwarded-For: 127.0.0.1".parse::<ureq::Header>().unwrap();
    /// assert!(header.is_name("x-forwarded-for"));
    /// ```
    pub fn is_name(&self, other: &str) -> bool {
        self.name().eq_ignore_ascii_case(other)
    }
}

impl FromStr for Header {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        //
        let line = AsciiString::from_str(s).map_err(|_| Error::BadHeader)?;
        let index = s.find(":").ok_or_else(|| Error::BadHeader)?;

        // no value?
        if index >= s.len() {
            return Err(Error::BadHeader);
        }

        Ok(Header { line, index })
    }
}

pub fn add_header(header: Header, headers: &mut Vec<Header>) {
    if !header.name().to_lowercase().starts_with("x-") {
        let name = header.name();
        headers.retain(|h| h.name() != name);
    }
    headers.push(header);
}
