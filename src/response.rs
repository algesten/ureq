use ascii::AsciiString;
use chunked_transfer;
use encoding::label::encoding_from_whatwg_label;
use encoding::DecoderTrap;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Result as IoResult;

use error::Error;

const DEFAULT_CONTENT_TYPE: &'static str = "text/plain";
const DEFAULT_CHARACTER_SET: &'static str = "utf-8";

pub struct Response {
    status_line: AsciiString,
    index: (usize, usize), // index into status_line where we split: HTTP/1.1 200 OK
    status: u16,
    headers: Vec<Header>,
    stream: Option<Stream>,
}

impl ::std::fmt::Debug for Response {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(
            f,
            "Response[status: {}, status_text: {}]",
            self.status(),
            self.status_text()
        )
    }
}

impl Response {
    /// The entire status line like: HTTP/1.1 200 OK
    pub fn status_line(&self) -> &str {
        self.status_line.as_str()
    }

    /// The http version: HTTP/1.1
    pub fn http_version(&self) -> &str {
        &self.status_line.as_str()[0..self.index.0]
    }

    /// The status as a u16: 200
    pub fn status(&self) -> &u16 {
        &self.status
    }

    /// The status text: OK
    pub fn status_text(&self) -> &str {
        &self.status_line.as_str()[self.index.1 + 1..].trim()
    }

    /// The header corresponding header value for the give name, if any.
    pub fn header<'a>(&self, name: &'a str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.is_name(name))
            .map(|h| h.value())
    }

    /// Tells if the response has the named header.
    pub fn has<'a>(&self, name: &'a str) -> bool {
        self.header(name).is_some()
    }

    /// All headers corresponding values for the give name, or empty vector.
    pub fn all<'a>(&self, name: &'a str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|h| h.is_name(name))
            .map(|h| h.value())
            .collect()
    }

    /// Whether the response status is: 200 <= status <= 299
    pub fn ok(&self) -> bool {
        self.status >= 200 && self.status <= 299
    }

    pub fn redirect(&self) -> bool {
        self.status >= 300 && self.status <= 399
    }

    /// Whether the response status is: 400 <= status <= 499
    pub fn client_error(&self) -> bool {
        self.status >= 400 && self.status <= 499
    }

    /// Whether the response status is: 500 <= status <= 599
    pub fn server_error(&self) -> bool {
        self.status >= 500 && self.status <= 599
    }

    /// Whether the response status is: 400 <= status <= 599
    pub fn error(&self) -> bool {
        self.client_error() || self.server_error()
    }

    /// The content type part of the "Content-Type" header without
    /// the charset.
    ///
    /// Example:
    ///
    /// ```
    /// let resp = ureq::get("https://www.google.com/").call();
    /// assert_eq!("text/html; charset=ISO-8859-1", resp.header("content-type").unwrap());
    /// assert_eq!("text/html", resp.content_type());
    /// ```
    pub fn content_type(&self) -> &str {
        self.header("content-type")
            .map(|header| {
                header
                    .find(";")
                    .map(|index| &header[0..index])
                    .unwrap_or(header)
            })
            .unwrap_or(DEFAULT_CONTENT_TYPE)
    }
    pub fn charset(&self) -> &str {
        self.header("content-type")
            .and_then(|header| {
                header.find(";").and_then(|semi| {
                    (&header[semi + 1..])
                        .find("=")
                        .map(|equal| (&header[semi + equal + 2..]).trim())
                })
            })
            .unwrap_or(DEFAULT_CHARACTER_SET)
    }

    pub fn into_reader(self) -> impl Read {
        let is_chunked = self.header("transfer-encoding")
            .map(|enc| enc.len() > 0) // whatever it says, do chunked
            .unwrap_or(false);
        let len = self.header("content-length").and_then(|l| l.parse::<usize>().ok());
        let reader = self.stream.expect("No reader in response?!");
        match is_chunked {
            true => Box::new(chunked_transfer::Decoder::new(reader)),
            false => {
                match len {
                    Some(len) => Box::new(LimitedRead::new(reader, len)),
                    None => Box::new(reader) as Box<Read>,
                }
            },
        }
    }

    pub fn into_string(self) -> IoResult<String> {
        let encoding = encoding_from_whatwg_label(self.charset())
            .or_else(|| encoding_from_whatwg_label(DEFAULT_CHARACTER_SET))
            .unwrap();
        let mut buf: Vec<u8> = vec![];
        self.into_reader().read_to_end(&mut buf)?;
        Ok(encoding.decode(&buf, DecoderTrap::Replace).unwrap())
    }

    pub fn into_json(self) -> IoResult<serde_json::Value> {
        let reader = self.into_reader();
        serde_json::from_reader(reader).map_err(|e| {
            IoError::new(
                ErrorKind::InvalidData,
                format!("Failed to read JSON: {}", e),
            )
        })
    }

    pub fn new(status: u16, status_text: &str, body: &str) -> Self {
        let r = format!("HTTP/1.1 {} {}\r\n\r\n{}\n", status, status_text, body);
        (r.as_ref() as &str)
            .parse::<Response>()
            .unwrap_or_else(|e| e.into())
    }

    pub fn from_read(reader: impl Read) -> Self
    {
        Self::do_from_read(reader).unwrap_or_else(|e| e.into())
    }

    fn do_from_read(mut reader: impl Read) -> Result<Response, Error>
    {
        //
        // HTTP/1.1 200 OK\r\n
        let status_line = read_next_line(&mut reader).map_err(|_| Error::BadStatus)?;

        let (index, status) = parse_status_line(status_line.as_str())?;

        let mut headers: Vec<Header> = Vec::new();
        loop {
            let line = read_next_line(&mut reader).map_err(|_| Error::BadHeader)?;
            if line.len() == 0 {
                break;
            }
            if let Ok(header) = line.as_str().parse::<Header>() {
                headers.push(header);
            }
        }

        Ok(Response {
            status_line,
            index,
            status,
            headers,
            stream: None,
        })
    }

    fn set_stream(&mut self, stream: Stream) {
        self.stream = Some(stream);
    }

    #[cfg(test)]
    pub fn to_write_vec(&self) -> Vec<u8> {
        self.stream.as_ref().unwrap().to_write_vec()
    }

}

fn parse_status_line(line: &str) -> Result<((usize, usize), u16), Error> {
    // HTTP/1.1 200 OK\r\n
    let mut split = line.splitn(3, ' ');

    let http_version = split.next().ok_or_else(|| Error::BadStatus)?;
    if http_version.len() < 5 {
        return Err(Error::BadStatus);
    }
    let index1 = http_version.len();

    let status = split.next().ok_or_else(|| Error::BadStatus)?;
    if status.len() < 3 {
        return Err(Error::BadStatus);
    }
    let index2 = index1 + status.len();

    let status = status.parse::<u16>().map_err(|_| Error::BadStatus)?;

    let status_text = split.next().ok_or_else(|| Error::BadStatus)?;
    if status_text.len() == 0 {
        return Err(Error::BadStatus);
    }

    Ok(((index1, index2), status))
}

impl FromStr for Response {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes().to_owned();
        let mut cursor = Cursor::new(bytes);
        let mut resp = Self::do_from_read(&mut cursor)?;
        resp.set_stream(Stream::Cursor(cursor));
        Ok(resp)
    }
}

impl Into<Response> for Error {
    fn into(self) -> Response {
        Response::new(self.status(), self.status_text(), &self.body_text())
    }
}

// application/x-www-form-urlencoded, application/json, and multipart/form-data

fn read_next_line<R: Read>(reader: &mut R) -> IoResult<AsciiString> {
    let mut buf = Vec::new();
    let mut prev_byte_was_cr = false;

    loop {
        let byte = reader.bytes().next();

        let byte = match byte {
            Some(b) => try!(b),
            None => return Err(IoError::new(ErrorKind::ConnectionAborted, "Unexpected EOF")),
        };

        if byte == b'\n' && prev_byte_was_cr {
            buf.pop(); // removing the '\r'
            return AsciiString::from_ascii(buf)
                .map_err(|_| IoError::new(ErrorKind::InvalidInput, "Header is not in ASCII"));
        }

        prev_byte_was_cr = byte == b'\r';

        buf.push(byte);
    }
}

struct LimitedRead {
    reader: Stream,
    limit: usize,
    position: usize,
}

impl LimitedRead {
    fn new(reader: Stream, limit: usize) -> Self {
        LimitedRead {
            reader,
            limit,
            position: 0,
        }
    }
}

impl Read for LimitedRead {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let left = self.limit - self.position;
        let from = if left < buf.len() {
            &mut buf[0..left]
        } else {
            buf
        };
        match self.reader.read(from) {
            Ok(amount) => {
                self.position += amount;
                Ok(amount)
            },
            Err(e) => Err(e)
        }
    }
}
