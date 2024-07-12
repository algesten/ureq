use std::fs::File;
use std::io::{self, Read, Stdin};
use std::net::TcpStream;

use crate::body::Body;

pub struct SendBody<'a> {
    inner: BodyInner<'a>,
    ended: bool,
}

impl<'a> SendBody<'a> {
    pub fn empty() -> SendBody<'static> {
        BodyInner::ByteSlice(&[]).into()
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
            BodyInner::ByteSlice(v) => {
                let max = v.len().min(buf.len());

                buf[..max].copy_from_slice(&v[..max]);
                *v = &v[max..];

                Ok(max)
            }
            BodyInner::Reader(v) => v.read(buf),
            BodyInner::OwnedReader(v) => v.read(buf),
        }?;

        if n == 0 {
            self.ended = true;
        }

        Ok(n)
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.ended
    }
}

mod private {
    pub trait Private {}
}
use http::Response;
use private::Private;

pub trait AsBody: Private {
    #[doc(hidden)]
    fn as_body(&mut self) -> SendBody;
}

pub(crate) enum BodyInner<'a> {
    ByteSlice(&'a [u8]),
    Reader(&'a mut dyn Read),
    OwnedReader(Box<dyn Read + Send + Sync>),
}

macro_rules! impl_into_body_slice {
    ($t:ty) => {
        impl Private for $t {}
        impl AsBody for $t {
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
        impl AsBody for $t {
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
impl AsBody for Body {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Reader(self).into()
    }
}

impl Private for Response<Body> {}
impl AsBody for Response<Body> {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Reader(self.body_mut()).into()
    }
}
