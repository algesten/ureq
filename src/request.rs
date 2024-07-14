use std::convert::TryFrom;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use http::{Method, Request, Response, Uri};

use crate::body::Body;
use crate::send_body::AsSendBody;
use crate::time::Instant;
use crate::util::private::Private;
use crate::{Agent, Error, SendBody};

/// Transparent wrapper around [`http::request::Builder`].
#[derive(Debug)]
pub struct RequestBuilder<MethodLimit> {
    agent: Agent,
    builder: http::request::Builder,
    _ph: PhantomData<MethodLimit>,
}

pub struct WithoutBody(());
impl Private for WithoutBody {}

pub struct WithBody(());
impl Private for WithBody {}

impl RequestBuilder<WithoutBody> {
    pub(crate) fn new<T>(agent: Agent, method: Method, uri: T) -> Self
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>,
    {
        Self {
            agent,
            builder: Request::builder().method(method).uri(uri),
            _ph: PhantomData,
        }
    }

    /// Sends the request with no body and blocks the caller until done.
    ///
    /// Use this with GET, HEAD, OPTIONS or TRACE. It sends neither
    /// Content-Length nor Transfer-Encoding.
    ///
    /// ```
    /// let resp = ureq::get("http://httpbin.org/get")
    ///     .call()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn call(self) -> Result<Response<Body>, Error> {
        let request = self.builder.body(())?;
        do_call(self.agent, request, SendBody::empty())
    }
}

impl RequestBuilder<WithBody> {
    pub(crate) fn new<T>(agent: Agent, method: Method, uri: T) -> Self
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>,
    {
        Self {
            agent,
            builder: Request::builder().method(method).uri(uri),
            _ph: PhantomData,
        }
    }

    /// Send data as bytes.
    ///
    /// The `Content-Length` header is implicitly set to the length of the serialized value.
    ///
    /// ```
    /// let resp = ureq::post("http://httpbin.org/put")
    ///     .send_bytes(&[0; 1000])?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn send_bytes(self, data: &[u8]) -> Result<Response<Body>, Error> {
        let request = self.builder.body(())?;
        let mut data_ref = data;
        do_call(self.agent, request, (&mut data_ref).as_body())
    }
}

fn do_call(agent: Agent, request: Request<()>, body: SendBody) -> Result<Response<Body>, Error> {
    let response = agent.do_run(request, body, Instant::now)?;
    Ok(response)
}

impl<MethodLimit> Deref for RequestBuilder<MethodLimit> {
    type Target = http::request::Builder;

    fn deref(&self) -> &Self::Target {
        &self.builder
    }
}

impl<MethodLimit> DerefMut for RequestBuilder<MethodLimit> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.builder
    }
}

// TODO(martin): implement reasonable Debug
