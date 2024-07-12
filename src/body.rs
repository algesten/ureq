use core::fmt;
use std::io::{self, Read};

use crate::pool::Connection;
use crate::time::Instant;
use crate::unit::{Event, Input, Unit};
use crate::Error;

pub struct Body {
    unit: Unit<()>,
    connection: Option<Connection>,
    info: ResponseInfo,
    current_time: Box<dyn Fn() -> Instant + Send + Sync>,
}

#[derive(Clone, Copy)]
pub(crate) struct ResponseInfo {
    content_encoding: ContentEncoding,
}

#[derive(Clone, Copy)]
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

        ResponseInfo { content_encoding }
    }
}

impl Body {
    pub(crate) fn new(
        unit: Unit<()>,
        connection: Connection,
        info: ResponseInfo,
        current_time: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        Body {
            unit,
            connection: Some(connection),
            info,
            current_time: Box::new(current_time),
        }
    }

    pub fn as_reader(&mut self, limit: u64) -> BodyReader {
        let info = self.info;
        BodyReader::new(LimitReader::shared(self, limit), info)
    }

    pub fn into_reader(self, limit: u64) -> BodyReader<'static> {
        let info = self.info;
        BodyReader::new(LimitReader::owned(self, limit), info)
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

    fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let now = (self.current_time)();

        let Some(connection) = &mut self.connection else {
            return Ok(0);
        };

        let event = self.unit.poll_event((self.current_time)())?;

        let timeout = match event {
            Event::AwaitInput { timeout } => timeout,
            Event::Reset { must_close } => {
                if let Some(connection) = self.connection.take() {
                    if must_close {
                        connection.close()
                    } else {
                        connection.reuse(now)
                    }
                }
                return Ok(0);
            }
            _ => unreachable!("Expected event AwaitInput"),
        };

        connection.await_input(timeout)?;
        let input = connection.buffers().input();

        let max = input.len().min(buf.len());
        let input = &input[..max];

        let input_used =
            self.unit
                .handle_input((self.current_time)(), Input::Data { input }, buf)?;

        connection.consume_input(input_used);

        let event = self.unit.poll_event((self.current_time)())?;

        let Event::ResponseBody { amount } = event else {
            unreachable!("Expected event ResponseBody");
        };

        Ok(amount)
    }
}

pub struct BodyReader<'a> {
    reader: ContentDecoder<'a>,
}

impl<'a> BodyReader<'a> {
    fn new(reader: LimitReader<'a>, info: ResponseInfo) -> BodyReader<'a> {
        let reader = match info.content_encoding {
            ContentEncoding::None => ContentDecoder::PassThrough(reader),
            #[cfg(feature = "gzip")]
            ContentEncoding::Gzip => {
                ContentDecoder::Gzip(flate2::read::MultiGzDecoder::new(reader))
            }
            #[cfg(not(feature = "gzip"))]
            ContentEncoding::Gzip => {
                info!("Not decompressing. Enable feature gzip");
                ContentDecoder::Gzip(reader)
            }
            #[cfg(feature = "brotli")]
            ContentEncoding::Brotli => {
                ContentDecoder::Brotli(brotli_decompressor::Decompressor::new(reader, 4096))
            }
            #[cfg(not(feature = "brotli"))]
            ContentEncoding::Brotli => {
                info!("Not decompressing. Enable feature brotli");
                ContentDecoder::Brotli(reader)
            }
            ContentEncoding::Unknown => {
                info!("Unknown content-encoding");
                ContentDecoder::PassThrough(reader)
            }
        };

        BodyReader { reader }
    }
}

impl<'a> Read for BodyReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

enum ContentDecoder<'a> {
    #[cfg(feature = "gzip")]
    Gzip(flate2::read::MultiGzDecoder<LimitReader<'a>>),
    #[cfg(not(feature = "gzip"))]
    Gzip(LimitReader<'a>),
    #[cfg(feature = "brotli")]
    Brotli(brotli_decompressor::Decompressor<LimitReader<'a>>),
    #[cfg(not(feature = "brotli"))]
    Brotli(LimitReader<'a>),
    PassThrough(LimitReader<'a>),
}

impl<'a> Read for ContentDecoder<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            ContentDecoder::Gzip(v) => v.read(buf),
            ContentDecoder::Brotli(v) => v.read(buf),
            ContentDecoder::PassThrough(v) => v.read(buf),
        }
    }
}

struct LimitReader<'a> {
    body: BodyRef<'a>,
    left: u64,
}

enum BodyRef<'a> {
    Shared(&'a mut Body),
    Owned(Body),
}

impl<'a> BodyRef<'a> {
    fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        match self {
            BodyRef::Shared(v) => v.do_read(buf),
            BodyRef::Owned(v) => v.do_read(buf),
        }
    }
}

impl<'a> LimitReader<'a> {
    fn shared(body: &'a mut Body, limit: u64) -> LimitReader<'a> {
        Self {
            body: BodyRef::Shared(body),
            left: limit,
        }
    }
}

impl LimitReader<'static> {
    fn owned(body: Body, limit: u64) -> LimitReader<'static> {
        Self {
            body: BodyRef::Owned(body),
            left: limit,
        }
    }
}

impl<'a> Read for LimitReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(Error::BodyExceedsLimit.into_io());
        }

        // The max buffer size is usize, which may be 32 bit.
        let max = (self.left.min(usize::MAX as u64) as usize).min(buf.len());

        let n = self
            .body
            .do_read(&mut buf[..max])
            .map_err(|e| e.into_io())?;

        self.left -= n as u64;

        Ok(n)
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
