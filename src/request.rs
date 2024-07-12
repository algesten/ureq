use std::convert::TryFrom;

use http::{HeaderName, HeaderValue, Method, Request, Response, Uri};

use crate::body::Body;
use crate::send_body::AsBody;
use crate::time::Instant;
use crate::{Agent, Error, SendBody};

#[derive(Debug)]
pub struct RequestBuilder {
    agent: Agent,
    builder: http::request::Builder,
}

impl RequestBuilder {
    pub(crate) fn new<T>(agent: Agent, method: Method, uri: T) -> Self
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>,
    {
        Self {
            agent,
            builder: Request::builder().method(method).uri(uri),
        }
    }

    /// Appends a header to this request builder.
    ///
    /// # Examples
    ///
    /// ```
    /// let res = ureq::get("https://httpbin.org/get")
    ///     .header("Accept", "text/html")
    ///     .header("X-Custom-Foo", "bar")
    ///     .call()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn header<K, V>(mut self, key: K, value: V) -> RequestBuilder
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.builder = self.builder.header(key, value);
        self
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
        let request = self.builder.body(()).unwrap();
        do_call(self.agent, request, SendBody::empty())
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
        let request = self.builder.body(()).unwrap();
        let mut data_ref = data;
        do_call(self.agent, request, (&mut data_ref).as_body())
    }
}

fn do_call(agent: Agent, request: Request<()>, body: SendBody) -> Result<Response<Body>, Error> {
    let response = agent.do_run(request, body, Instant::now)?;
    Ok(response)
}

// TODO(martin): implement reasonable Debug
// TODO(martin): ureq 2.x implements Clone for Request
