use crate::stream::Stream;
use chunked_transfer;
use std::io::{copy, empty, Cursor, Read, Result as IoResult};

#[cfg(feature = "charset")]
use crate::response::DEFAULT_CHARACTER_SET;
#[cfg(feature = "charset")]
use encoding::label::encoding_from_whatwg_label;
#[cfg(feature = "charset")]
use encoding::EncoderTrap;

#[cfg(feature = "json")]
use super::SerdeValue;
#[cfg(feature = "json")]
use serde_json;

/// The different kinds of bodies to send.
///
/// *Internal API*
pub(crate) enum Payload {
    Empty,
    Text(String, String),
    #[cfg(feature = "json")]
    JSON(SerdeValue),
    Reader(Box<dyn Read + 'static>),
    Bytes(Vec<u8>),
}

impl ::std::fmt::Debug for Payload {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        match self {
            Payload::Empty => write!(f, "Empty"),
            Payload::Text(t, _) => write!(f, "{}", t),
            #[cfg(feature = "json")]
            Payload::JSON(_) => write!(f, "JSON"),
            Payload::Reader(_) => write!(f, "Reader"),
            Payload::Bytes(v) => write!(f, "{:?}", v),
        }
    }
}

impl Default for Payload {
    fn default() -> Payload {
        Payload::Empty
    }
}

/// Payloads are turned into this type where we can hold both a size and the reader.
///
/// *Internal API*
pub(crate) struct SizedReader {
    pub size: Option<usize>,
    pub reader: Box<dyn Read + 'static>,
}

impl ::std::fmt::Debug for SizedReader {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(f, "SizedReader[size={:?},reader]", self.size)
    }
}

impl SizedReader {
    fn new(size: Option<usize>, reader: Box<dyn Read + 'static>) -> Self {
        SizedReader { size, reader }
    }
}

impl Payload {
    pub fn into_read(self) -> SizedReader {
        match self {
            Payload::Empty => SizedReader::new(None, Box::new(empty())),
            Payload::Text(text, _charset) => {
                #[cfg(feature = "charset")]
                let bytes = {
                    let encoding = encoding_from_whatwg_label(&_charset)
                        .or_else(|| encoding_from_whatwg_label(DEFAULT_CHARACTER_SET))
                        .unwrap();
                    encoding.encode(&text, EncoderTrap::Replace).unwrap()
                };
                #[cfg(not(feature = "charset"))]
                let bytes = text.into_bytes();
                let len = bytes.len();
                let cursor = Cursor::new(bytes);
                SizedReader::new(Some(len), Box::new(cursor))
            }
            #[cfg(feature = "json")]
            Payload::JSON(v) => {
                let bytes = serde_json::to_vec(&v).expect("Bad JSON in payload");
                let len = bytes.len();
                let cursor = Cursor::new(bytes);
                SizedReader::new(Some(len), Box::new(cursor))
            }
            Payload::Reader(read) => SizedReader::new(None, read),
            Payload::Bytes(bytes) => {
                let len = bytes.len();
                let cursor = Cursor::new(bytes);
                SizedReader::new(Some(len), Box::new(cursor))
            }
        }
    }
}

/// Helper to send a body, either as chunked or not.
pub(crate) fn send_body(
    mut body: SizedReader,
    do_chunk: bool,
    stream: &mut Stream,
) -> IoResult<u64> {
    let n = if do_chunk {
        let mut chunker = chunked_transfer::Encoder::new(stream);
        copy(&mut body.reader, &mut chunker)?
    } else {
        copy(&mut body.reader, stream)?
    };

    Ok(n)
}
