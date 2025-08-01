use std::fs::File;
use std::io::{self, Read, Stdin};
use std::net::TcpStream;

use crate::body::{Body, BodyReader};
use crate::http;
use crate::util::private::Private;

/// Request body for sending data via POST, PUT and PATCH.
///
/// Typically not interacted with directly since the trait [`AsSendBody`] is implemented
/// for the majority of the types of data a user might want to send to a remote server.
/// That means if you want to send things like `String`, `&str` or `[u8]`, they can be
/// used directly. See documentation for [`AsSendBody`].
///
/// The exception is when using [`Read`] trait bodies, in which case we wrap the request
/// body directly. See below [`SendBody::from_reader`].
///
pub struct SendBody<'a> {
    inner: BodyInner<'a>,
    ended: bool,
}

impl<'a> SendBody<'a> {
    /// Creates an empty body.
    pub fn none() -> SendBody<'static> {
        BodyInner::None.into()
    }

    /// Creates a body from a shared [`Read`] impl.
    pub fn from_reader(reader: &'a mut dyn Read) -> SendBody<'a> {
        BodyInner::Reader(reader).into()
    }

    /// Creates a body from an owned [`Read`] impl.
    pub fn from_owned_reader(reader: impl Read + 'static) -> SendBody<'static> {
        BodyInner::OwnedReader(Box::new(reader)).into()
    }

    /// Creates a body to send as JSON from any [`Serialize`](serde::ser::Serialize) value.
    #[cfg(feature = "json")]
    pub fn from_json(
        value: &impl serde::ser::Serialize,
    ) -> Result<SendBody<'static>, crate::Error> {
        let json = serde_json::to_vec_pretty(value)?;
        Ok(BodyInner::ByteVec(io::Cursor::new(json)).into())
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
            #[cfg(feature = "json")]
            BodyInner::ByteVec(v) => v.read(buf),
            BodyInner::Reader(v) => v.read(buf),
            BodyInner::OwnedReader(v) => v.read(buf),
            BodyInner::Body(v) => v.read(buf),
        }?;

        if n == 0 {
            self.ended = true;
        }

        Ok(n)
    }

    pub(crate) fn body_mode(&self) -> BodyMode {
        self.inner.body_mode()
    }

    /// Turn this `SendBody` into a reader.
    ///
    /// This is useful in [`Middleware`][crate::middleware::Middleware] to make changes to the
    /// body before sending it.
    ///
    /// ```
    /// use ureq::{SendBody, Body};
    /// use ureq::middleware::MiddlewareNext;
    /// use ureq::http::{Request, Response, header::HeaderValue};
    /// use std::io::Read;
    ///
    /// fn my_middleware(req: Request<SendBody>, next: MiddlewareNext)
    ///     -> Result<Response<Body>, ureq::Error> {
    ///
    ///     // Take apart the request.
    ///     let (parts, body) = req.into_parts();
    ///
    ///     // Take the first 100 bytes of the incoming send body.
    ///     let mut reader = body.into_reader().take(100);
    ///
    ///     // Create a new SendBody.
    ///     let new_body = SendBody::from_reader(&mut reader);
    ///
    ///     // Reconstitute the request.
    ///     let req = Request::from_parts(parts, new_body);
    ///
    ///     // set my bespoke header and continue the chain
    ///     next.handle(req)
    /// }
    /// ```
    pub fn into_reader(self) -> impl Sized + io::Read + 'a {
        ReadAdapter(self)
    }
}

struct ReadAdapter<'a>(SendBody<'a>);

impl<'a> io::Read for ReadAdapter<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

use http::Response;
use ureq_proto::BodyMode;

/// Trait for common types to send in POST, PUT or PATCH.
///
/// Sending common data types such as `String`, `&str` or `&[u8]` require no further wrapping
/// and can be sent either by [`RequestBuilder::send()`][crate::RequestBuilder::send] or using the
/// `http` crate [`Request`][http::Request] directly (see example below).
///
/// Implemented for:
///
/// * `&str`
/// * `&String`
/// * `&Vec<u8>`
/// * `&File`
/// * `&TcpStream`
/// * `&[u8]`
/// * `Response<Body>`
/// * `String`
/// * `Vec<u8>`
/// * `File`
/// * `Stdin`
/// * `TcpStream`
/// * `UnixStream` (not on windows)
/// * `&[u8; N]`
/// * `()`
///
/// # Example
///
/// These two examples are equivalent.
///
/// ```
/// let data: &[u8] = b"My special request body data";
///
/// let response = ureq::post("https://httpbin.org/post")
///     .send(data)?;
/// # Ok::<_, ureq::Error>(())
/// ```
///
/// Using `http` crate API
///
/// ```
/// use ureq::http;
///
/// let data: &[u8] = b"My special request body data";
///
/// let request = http::Request::post("https://httpbin.org/post")
///     .body(data)?;
///
/// let response = ureq::run(request)?;
/// # Ok::<_, ureq::Error>(())
/// ```
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
                #[cfg(feature = "json")]
                BodyInner::ByteVec(v) => BodyInner::ByteSlice(v.get_ref()),
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
    #[cfg(feature = "json")]
    ByteVec(io::Cursor<Vec<u8>>),
    Body(Box<BodyReader<'a>>),
    Reader(&'a mut dyn Read),
    OwnedReader(Box<dyn Read>),
}

impl<'a> BodyInner<'a> {
    pub fn body_mode(&self) -> BodyMode {
        match self {
            BodyInner::None => BodyMode::NoBody,
            BodyInner::ByteSlice(v) => BodyMode::LengthDelimited(v.len() as u64),
            #[cfg(feature = "json")]
            BodyInner::ByteVec(v) => BodyMode::LengthDelimited(v.get_ref().len() as u64),
            BodyInner::Body(v) => v.body_mode(),
            BodyInner::Reader(_) => BodyMode::Chunked,
            BodyInner::OwnedReader(_) => BodyMode::Chunked,
        }
    }
}

impl Private for &[u8] {}
impl AsSendBody for &[u8] {
    fn as_body(&mut self) -> SendBody {
        BodyInner::ByteSlice(self).into()
    }
}

impl Private for &str {}
impl AsSendBody for &str {
    fn as_body(&mut self) -> SendBody {
        BodyInner::ByteSlice((*self).as_ref()).into()
    }
}

impl Private for String {}
impl AsSendBody for String {
    fn as_body(&mut self) -> SendBody {
        BodyInner::ByteSlice((*self).as_ref()).into()
    }
}

impl Private for Vec<u8> {}
impl AsSendBody for Vec<u8> {
    fn as_body(&mut self) -> SendBody {
        BodyInner::ByteSlice((*self).as_ref()).into()
    }
}

impl Private for &String {}
impl AsSendBody for &String {
    fn as_body(&mut self) -> SendBody {
        BodyInner::ByteSlice((*self).as_ref()).into()
    }
}

impl Private for &Vec<u8> {}
impl AsSendBody for &Vec<u8> {
    fn as_body(&mut self) -> SendBody {
        BodyInner::ByteSlice((*self).as_ref()).into()
    }
}

impl Private for &File {}
impl AsSendBody for &File {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Reader(self).into()
    }
}

impl Private for &TcpStream {}
impl AsSendBody for &TcpStream {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Reader(self).into()
    }
}

impl Private for File {}
impl AsSendBody for File {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Reader(self).into()
    }
}

impl Private for TcpStream {}
impl AsSendBody for TcpStream {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Reader(self).into()
    }
}

impl Private for Stdin {}
impl AsSendBody for Stdin {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Reader(self).into()
    }
}

// MSRV 1.78
// impl_into_body!(&Stdin, Reader);

#[cfg(target_family = "unix")]
use std::os::unix::net::UnixStream;

#[cfg(target_family = "unix")]
impl Private for UnixStream {}
#[cfg(target_family = "unix")]
impl AsSendBody for UnixStream {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Reader(self).into()
    }
}

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
        BodyInner::Body(Box::new(self.as_reader())).into()
    }
}

impl Private for Response<Body> {}
impl AsSendBody for Response<Body> {
    fn as_body(&mut self) -> SendBody {
        BodyInner::Body(Box::new(self.body_mut().as_reader())).into()
    }
}

impl<const N: usize> Private for &[u8; N] {}
impl<const N: usize> AsSendBody for &[u8; N] {
    fn as_body(&mut self) -> SendBody {
        BodyInner::ByteSlice(self.as_slice()).into()
    }
}

impl Private for () {}
impl AsSendBody for () {
    fn as_body(&mut self) -> SendBody {
        BodyInner::None.into()
    }
}
