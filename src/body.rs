use chunked_transfer;
use std::io::{empty, Cursor, Read, Result as IoResult, Write};
use stream::Stream;

#[cfg(feature = "charset")]
use encoding::label::encoding_from_whatwg_label;
#[cfg(feature = "charset")]
use encoding::EncoderTrap;
#[cfg(feature = "charset")]
use response::DEFAULT_CHARACTER_SET;

#[cfg(feature = "json")]
use super::SerdeValue;
#[cfg(feature = "json")]
use serde_json;

const CHUNK_SIZE: usize = 1024 * 1024;

pub enum Payload {
    Empty,
    Text(String, String),
    #[cfg(feature = "json")]
    JSON(SerdeValue),
    Reader(Box<Read + 'static>),
}

impl Default for Payload {
    fn default() -> Payload {
        Payload::Empty
    }
}

pub struct SizedReader {
    pub size: Option<usize>,
    pub reader: Box<Read + 'static>,
}

impl SizedReader {
    fn new(size: Option<usize>, reader: Box<Read + 'static>) -> Self {
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
        }
    }
}

pub fn send_body(body: SizedReader, do_chunk: bool, stream: &mut Stream) -> IoResult<()> {
    if do_chunk {
        pipe(body.reader, chunked_transfer::Encoder::new(stream))?;
    } else {
        pipe(body.reader, stream)?;
    }

    Ok(())
}

fn pipe<R, W>(mut reader: R, mut writer: W) -> IoResult<()>
where
    R: Read,
    W: Write,
{
    let mut buf = [0_u8; CHUNK_SIZE];
    loop {
        let len = reader.read(&mut buf)?;
        if len == 0 {
            break;
        }
        writer.write_all(&buf[0..len])?;
    }
    Ok(())
}
