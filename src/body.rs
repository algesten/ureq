use crate::stream::Stream;
use std::io::{copy, empty, Cursor, Read, Write, Result as IoResult};

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

const CHUNK_MAX_SIZE: usize = 0x4000;   // Maximum size of a TLS fragment
const CHUNK_HEADER_MAX_SIZE: usize = 6; // four hex digits plus "\r\n"
const CHUNK_FOOTER_SIZE: usize = 2;     // "\r\n"
const CHUNK_MAX_PAYLOAD_SIZE: usize = CHUNK_MAX_SIZE - CHUNK_HEADER_MAX_SIZE - CHUNK_FOOTER_SIZE;


// copy_chunks() improves over chunked_transfer's Encoder + io::copy with the
// following performance optimizations:
// 1) It avoid copying memory.
// 2) chunked_transfer's Encoder issues 4 separate write() per chunk. This is costly
//    overhead. Instead, we do a single write() per chunk.
// The measured benefit on a Linux machine is a 50% reduction in CPU usage on a https connection.
fn copy_chunked<R: Read, W: Write>(reader: &mut R, writer: &mut W) -> IoResult<u64> {
    // The chunk layout is:
    // header:header_max_size | payload:max_payload_size | footer:footer_size
    let mut chunk = Vec::with_capacity(CHUNK_MAX_SIZE);
    let mut written = 0;
    loop {
        // We first read the payload
        chunk.resize(CHUNK_HEADER_MAX_SIZE, 0);
        let payload_size = reader.take(CHUNK_MAX_PAYLOAD_SIZE as u64).read_to_end(&mut chunk)?;

        // Then write the header
        let header_str = format!("{:x}\r\n", payload_size);
        let header = header_str.as_bytes();
        assert!(header.len() <= CHUNK_HEADER_MAX_SIZE);
        let start_index = CHUNK_HEADER_MAX_SIZE - header.len();
        (&mut chunk[start_index..]).write(&header).unwrap();

        // And add the footer
        chunk.extend_from_slice(b"\r\n");

        // Finally Write the chunk
        writer.write_all(&chunk[start_index..])?;
        written += payload_size as u64;

        // On EOF, we wrote a 0 sized chunk. This is what the chunked encoding protocol requires.
        if payload_size == 0 {
            return Ok(written);
        }
    }
}

#[test]
fn test_copy_chunked() {
    let mut source = Vec::<u8>::new();
    source.resize(CHUNK_MAX_PAYLOAD_SIZE, 33);
    source.extend_from_slice(b"hello world");

    let mut dest = Vec::<u8>::new();
    copy_chunked(&mut &source[..], &mut dest).unwrap();

    let mut dest_expected = Vec::<u8>::new();
    dest_expected.extend_from_slice(format!("{:x}\r\n", CHUNK_MAX_PAYLOAD_SIZE).as_bytes());
    dest_expected.resize(dest_expected.len() + CHUNK_MAX_PAYLOAD_SIZE, 33);
    dest_expected.extend_from_slice(b"\r\n");

    dest_expected.extend_from_slice(b"b\r\nhello world\r\n");
    dest_expected.extend_from_slice(b"0\r\n\r\n");

    assert_eq!(dest, dest_expected);
}

/// Helper to send a body, either as chunked or not.
pub(crate) fn send_body(
    mut body: SizedReader,
    do_chunk: bool,
    stream: &mut Stream,
) -> IoResult<u64> {
    let n = if do_chunk {
        copy_chunked(&mut body.reader, stream)?
    } else {
        copy(&mut body.reader, stream)?
    };

    Ok(n)
}
