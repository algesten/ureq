use std::fs::File;
use std::io::{self, Read, Stdin};
use std::net::TcpStream;

use crate::body::{Body, BodyReader};
use crate::util::private::Private;

pub struct SendBody<'a> {
    inner: BodyInner<'a>,
    ended: bool,
}

impl<'a> SendBody<'a> {
    pub fn none() -> SendBody<'static> {
        BodyInner::None.into()
    }

    pub fn from_reader(reader: &'a mut dyn Read) -> SendBody<'a> {
        BodyInner::Reader(reader).into()
    }

    pub fn from_owned_reader<R>(reader: R) -> SendBody<'static>
    where
        R: Read + Send + Sync + 'static,
    {
        BodyInner::OwnedReader(Box::new(reader)).into()
    }

    pub(crate) fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = match &mut self.inner {
            BodyInner::None => {
                return Ok(0);
            }
            BodyInner::ByteSlice(v) => {
                let max = v.len().min(buf.len());

                buf[..max].copy_from_slice(&v[..max]);
                *v = &v[max..];

                Ok(max)
            }
            BodyInner::Reader(v) => v.read(buf),
            BodyInner::OwnedReader(v) => v.read(buf),
            BodyInner::Body(v) => v.read(buf),
        }?;

        if n == 0 {
            self.ended = true;
        }

        Ok(n)
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.ended
    }

    pub(crate) fn body_mode(&self) -> BodyMode {
        self.inner.body_mode()
    }
}

use hoot::BodyMode;
use http::Response;

pub trait AsSendBody: Private {
    #[doc(hidden)]
    fn as_body(&mut self) -> SendBody;
}

impl<'a> Private for SendBody<'a> {}
impl<'a> AsSendBody for SendBody<'a> {
    fn as_body(&mut self) -> SendBody {
        SendBody {
            inner: match &mut self.inner {
                BodyInner::None => BodyInner::None,
                BodyInner::ByteSlice(v) => BodyInner::ByteSlice(v),
                BodyInner::Reader(v) => BodyInner::Reader(v),
                BodyInner::Body(v) => BodyInner::Reader(v),
                BodyInner::OwnedReader(v) => BodyInner::Reader(v),
            },
            ended: self.ended,
        }
    }
}

pub(crate) enum BodyInner<'a> {
    None,
    ByteSlice(&'a [u8]),
    Body(BodyReader<'a>),
    Reader(&'a mut dyn Read),
    OwnedReader(Box<dyn Read + Send + Sync>),
}

impl<'a> BodyInner<'a> {
    pub fn body_mode(&self) -> BodyMode {
        match self {
            BodyInner::None => BodyMode::NoBody,
            BodyInner::ByteSlice(v) => BodyMode::LengthDelimited(v.len() as u64),
            BodyInner::Body(v) => v.body_mode(),
            BodyInner::Reader(_) => BodyMode::Chunked,
            BodyInner::OwnedReader(_) => BodyMode::Chunked,
        }
    }
}

macro_rules! impl_into_body_slice {
    ($t:ty) => {
        impl Private for $t {}
        impl AsSendBody for $t {
            fn as_body(&mut self) -> SendBody {
                BodyInner::ByteSlice((*self).as_ref()).into()
            }
        }
    };
}

impl_into_body_slice!(&[u8]);
impl_into_body_slice!(&str);
impl_into_body_slice!(String);
impl_into_body_slice!(Vec<u8>);
impl_into_body_slice!(&String);
impl_into_body_slice!(&Vec<u8>);

macro_rules! impl_into_body {
    ($t:ty, $s:tt) => {
        impl Private for $t {}
        impl AsSendBody for $t {
            fn as_body(&mut self) -> SendBody {
                BodyInner::$s(self).into()
            }
        }
    };
}

impl_into_body!(&File, Reader);
impl_into_body!(&TcpStream, Reader);
impl_into_body!(File, Reader);
impl_into_body!(TcpStream, Reader);
impl_into_body!(Stdin, Reader);

// MSRV 1.78
// impl_into_body!(&Stdin, Reader);

#[cfg(target_family = "unix")]
use std::os::unix::net::UnixStream;

#[cfg(target_family = "unix")]
impl_into_body!(UnixStream, Reader);

impl<'a> From<BodyInner<'a>> for SendBody<'a> {
    fn from(inner: BodyInner<'a>) -> Self {
        SendBody {
            inner,
            ended: false,
        }
    }
}

impl Private for Body {}
impl AsSendBody for Body {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Body(self.as_reader(u64::MAX)).into()
    }
}

impl Private for Response<Body> {}
impl AsSendBody for Response<Body> {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Body(self.body_mut().as_reader(u64::MAX)).into()
    }
}

impl<const N: usize> Private for &[u8; N] {}
impl<const N: usize> AsSendBody for &[u8; N] {
    fn as_body(&mut self) -> SendBody {
        BodyInner::ByteSlice(self.as_slice()).into()
    }
}
