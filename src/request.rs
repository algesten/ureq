use std::convert::TryFrom;

use http::{HeaderName, HeaderValue, Method, Request, Response, Uri};

use crate::body::{BodyOwned, RecvBody};
use crate::{Agent, Body, Error};

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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let res = ureq::get("https://host.test/my-path")
    ///     .header("Accept", "text/html")
    ///     .header("X-Custom-Foo", "bar")
    ///     .call()?;
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::get("http://httpbin.org/get")
    ///     .call()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn call(self) -> Result<Response<RecvBody>, Error> {
        let request = self.builder.body(BodyOwned::empty()).unwrap();
        do_call(self.agent, request)
    }

    /// Send data as bytes.
    ///
    /// The `Content-Length` header is implicitly set to the length of the serialized value.
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let resp = ureq::post("http://httpbin.org/put")
    ///     .send_bytes(&[0; 1000])?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn send_bytes(self, data: &[u8]) -> Result<Response<RecvBody>, Error> {
        let request = self.builder.body(data).unwrap();
        do_call(self.agent, request)
    }
}

fn do_call(mut agent: Agent, request: Request<impl Body>) -> Result<Response<RecvBody>, Error> {
    let response = agent.run(&request)?;
    Ok(response)
}

// TODO(martin): implement reasonable Debug
// TODO(martin): ureq 2.x implements Clone for Request
