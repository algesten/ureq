use core::fmt;
use std::io::{self, Read};

use crate::pool::Connection;
use crate::time::Instant;
use crate::unit::Unit;
use crate::Error;

use self::handler::{UnitHandler, UnitHandlerRef};
use self::limit::LimitReader;

mod handler;
mod limit;

#[cfg(feature = "charset")]
mod charset;

#[cfg(feature = "gzip")]
mod gzip;

#[cfg(feature = "brotli")]
mod brotli;

pub struct Body {
    info: ResponseInfo,
    unit_handler: UnitHandler,
}

#[derive(Clone)]
pub(crate) struct ResponseInfo {
    content_encoding: ContentEncoding,
    mime_type: Option<String>,
    charset: Option<String>,
}

impl Body {
    pub(crate) fn new(
        unit: Unit<()>,
        connection: Connection,
        info: ResponseInfo,
        current_time: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        Body {
            info,
            unit_handler: UnitHandler::new(unit, connection, current_time),
        }
    }

    pub fn mime_type(&self) -> Option<&str> {
        self.info.mime_type.as_deref()
    }

    pub fn charset(&self) -> Option<&str> {
        self.info.charset.as_deref()
    }

    pub fn as_reader(&mut self, limit: u64) -> BodyReader {
        BodyReader::new(
            LimitReader::new(UnitHandlerRef::Shared(&mut self.unit_handler), limit),
            &self.info,
        )
    }

    pub fn into_reader(self, limit: u64) -> BodyReader<'static> {
        BodyReader::new(
            LimitReader::new(UnitHandlerRef::Owned(self.unit_handler), limit),
            &self.info,
        )
    }

    pub fn read_to_string(&mut self, limit: usize) -> Result<String, Error> {
        let mut buf = String::new();
        let mut reader = self.as_reader(limit as u64);
        reader.read_to_string(&mut buf)?;
        Ok(buf)
    }

    pub fn read_to_vec(&mut self, limit: usize) -> Result<Vec<u8>, Error> {
        let mut buf = Vec::new();
        let mut reader = self.as_reader(limit as u64);
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

#[derive(Debug, Clone, Copy)]
enum ContentEncoding {
    None,
    Gzip,
    Brotli,
    Unknown,
}

impl ResponseInfo {
    pub fn new(headers: &http::HeaderMap) -> Self {
        let content_encoding = headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(ContentEncoding::from)
            .unwrap_or(ContentEncoding::None);

        let (mime_type, charset) = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(split_content_type)
            .unwrap_or((None, None));

        ResponseInfo {
            content_encoding,
            mime_type,
            charset,
        }
    }
}

fn split_content_type(content_type: &str) -> (Option<String>, Option<String>) {
    // Content-Type: text/plain; charset=iso-8859-1
    let mut split = content_type.split(';');

    let Some(mime_type) = split.next() else {
        return (None, None);
    };

    let mut charset = None;

    for maybe_charset in split {
        let maybe_charset = maybe_charset.trim();
        if let Some(s) = maybe_charset.strip_prefix("charset=") {
            charset = Some(s.to_string());
        }
    }

    (Some(mime_type.to_string()), charset)
}

pub struct BodyReader<'a> {
    reader: CharsetDecoder<ContentDecoder<LimitReader<UnitHandlerRef<'a>>>>,
}

impl<'a> BodyReader<'a> {
    fn new(reader: LimitReader<UnitHandlerRef<'a>>, info: &ResponseInfo) -> BodyReader<'a> {
        let reader = match info.content_encoding {
            ContentEncoding::None | ContentEncoding::Unknown => ContentDecoder::PassThrough(reader),
            #[cfg(feature = "gzip")]
            ContentEncoding::Gzip => ContentDecoder::Gzip(Box::new(gzip::GzipDecoder::new(reader))),
            #[cfg(not(feature = "gzip"))]
            ContentEncoding::Gzip => ContentDecoder::PassThrough(reader),
            #[cfg(feature = "brotli")]
            ContentEncoding::Brotli => {
                ContentDecoder::Brotli(Box::new(brotli::BrotliDecoder::new(reader)))
            }
            #[cfg(not(feature = "brotli"))]
            ContentEncoding::Brotli => ContentDecoder::PassThrough(reader),
        };

        let reader = charset_decoder(reader, info.mime_type.as_deref(), info.charset.as_deref());

        BodyReader { reader }
    }
}

#[allow(unused)]
fn charset_decoder<R: Read>(
    reader: R,
    mime_type: Option<&str>,
    charset: Option<&str>,
) -> CharsetDecoder<R> {
    let is_text = mime_type.map(|m| m.starts_with("text/")).unwrap_or(false);

    if !is_text {
        return CharsetDecoder::PassThrough(reader);
    }

    #[cfg(feature = "charset")]
    {
        use encoding_rs::{Encoding, UTF_8};

        let from = charset
            .and_then(|c| Encoding::for_label(c.as_bytes()))
            .unwrap_or(UTF_8);

        if from == UTF_8 {
            // Do nothing
            CharsetDecoder::PassThrough(reader)
        } else {
            CharsetDecoder::Decoder(self::charset::CharCodec::new(reader, from, UTF_8))
        }
    }

    #[cfg(not(feature = "charset"))]
    {
        CharsetDecoder::PassThrough(reader)
    }
}

impl<'a> Read for BodyReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

enum CharsetDecoder<R> {
    #[cfg(feature = "charset")]
    Decoder(charset::CharCodec<R>),
    PassThrough(R),
}

impl<R: io::Read> Read for CharsetDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(feature = "charset")]
            CharsetDecoder::Decoder(v) => v.read(buf),
            CharsetDecoder::PassThrough(v) => v.read(buf),
        }
    }
}

enum ContentDecoder<R: io::Read> {
    #[cfg(feature = "gzip")]
    Gzip(Box<gzip::GzipDecoder<R>>),
    #[cfg(feature = "brotli")]
    Brotli(Box<brotli::BrotliDecoder<R>>),
    PassThrough(R),
}

impl<R: Read> Read for ContentDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(feature = "gzip")]
            ContentDecoder::Gzip(v) => v.read(buf),
            #[cfg(feature = "brotli")]
            ContentDecoder::Brotli(v) => v.read(buf),
            ContentDecoder::PassThrough(v) => v.read(buf),
        }
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Body").finish()
    }
}

impl From<&str> for ContentEncoding {
    fn from(s: &str) -> Self {
        match s {
            "gzip" => ContentEncoding::Gzip,
            "br" => ContentEncoding::Brotli,
            _ => {
                info!("Unknown content-encoding: {}", s);
                ContentEncoding::Unknown
            }
        }
    }
}
