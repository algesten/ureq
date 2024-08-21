use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use http::{HeaderName, HeaderValue, Method, Request, Response, Uri, Version};

use crate::body::Body;
use crate::send_body::AsSendBody;
use crate::util::private::Private;
use crate::{Agent, Error, SendBody, Timeouts};

/// Transparent wrapper around [`http::request::Builder`].
///
/// The purpose is to provide the [`.call()`][RequestBuilder::call] and [`.send()`][RequestBuilder::send]
/// functions to make a simpler API for sending requests.
pub struct RequestBuilder<B> {
    agent: Agent,
    builder: http::request::Builder,
    _ph: PhantomData<B>,
}

#[derive(Debug)]
pub struct WithoutBody(());
impl Private for WithoutBody {}

#[derive(Debug)]
pub struct WithBody(());
impl Private for WithBody {}

impl<Any> RequestBuilder<Any> {
    /// Appends a header to this request builder.
    ///
    /// This function will append the provided key/value as a header to the
    /// set of headers. It does not replace headers.
    ///
    /// # Examples
    ///
    /// ```
    /// let req = ureq::get("https://httpbin.org/get")
    ///     .header("X-Custom-Foo", "bar");
    /// ```
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.builder = self.builder.header(key, value);
        self
    }

    /// Overrides the URI for this request.
    ///
    /// Typically this is set via `ureq::get(<uri>)` or `Agent::get(<uri>)`. This
    /// lets us change it.
    ///
    /// # Examples
    ///
    /// ```
    /// let req = ureq::get("https://www.google.com/")
    ///     .uri("https://httpbin.org/get");
    /// ```
    pub fn uri<T>(mut self, uri: T) -> Self
    where
        Uri: TryFrom<T>,
        <Uri as TryFrom<T>>::Error: Into<http::Error>,
    {
        self.builder = self.builder.uri(uri);
        self
    }

    /// Set the HTTP version for this request.
    ///
    /// By default this is HTTP/1.1.
    /// ureq only handles HTTP/1.1 and HTTP/1.0.
    ///
    /// # Examples
    ///
    /// ```
    /// use ureq::http::Version;
    ///
    /// let req = ureq::get("https://www.google.com/")
    ///     .version(Version::HTTP_10);
    /// ```
    pub fn version(mut self, version: Version) -> Self {
        self.builder = self.builder.version(version);
        self
    }

    /// Override agent timeouts on the request level.
    ///
    /// The agent setting is copied and modified on request level.
    ///
    /// # Example
    ///
    /// ```
    /// use ureq::{Agent, AgentConfig, Timeouts};
    /// use std::time::Duration;
    ///
    /// let agent: Agent = AgentConfig {
    ///     timeouts: Timeouts {
    ///         global: Some(Duration::from_secs(10)),
    ///        ..Default::default()
    ///     },
    ///     ..Default::default()
    /// }.into();
    ///
    /// let mut builder = agent.get("https://httpbin.org/get");
    ///
    /// // This clones the timeouts from agent level to request level.
    /// let timeouts = builder.timeouts();
    ///
    /// assert_eq!(timeouts.global, Some(Duration::from_secs(10)));
    ///
    /// // Override the global timeout on the request level.
    /// timeouts.global = Some(Duration::from_secs(3));
    ///
    /// // Make the request
    /// let response = builder.call()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn timeouts(&mut self) -> &mut Timeouts {
        let exts = self
            .builder
            .extensions_mut()
            .expect("builder without errors");

        if exts.get::<Timeouts>().is_none() {
            exts.insert(self.agent.config().timeouts);
        }

        // unwrap is ok because of above logic
        exts.get_mut().unwrap()
    }
}

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

    /// Sends the request and blocks the caller until we receive a response.
    ///
    /// It sends neither `Content-Length` nor `Transfer-Encoding`.
    ///
    /// ```
    /// let res = ureq::get("http://httpbin.org/get")
    ///     .call()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn call(self) -> Result<Response<Body>, Error> {
        let request = self.builder.body(())?;
        do_call(self.agent, request, SendBody::none())
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

    /// Set the content-type header.
    ///
    /// ```
    /// let res = ureq::post("http://httpbin.org/post")
    ///     .content_type("text/html; charset=utf-8")
    ///     .send("<html><body>åäö</body></html>")?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn content_type<V>(mut self, content_type: V) -> Self
    where
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.builder = self.builder.header("content-type", content_type);
        self
    }

    /// Send body data and blocks the caller until we receive response.
    ///
    /// ```
    /// let res = ureq::post("http://httpbin.org/post")
    ///     .send(&[0_u8; 1000])?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn send(self, data: impl AsSendBody) -> Result<Response<Body>, Error> {
        let request = self.builder.body(())?;
        let mut data_ref = data;
        do_call(self.agent, request, data_ref.as_body())
    }

    /// Send body data as JSON.
    ///
    /// Requires the **json** feature.
    ///
    /// The data typically derives [`Serialize`](serde::Serialize) and is converted
    /// to a string before sending (does allocate).
    ///
    /// ```
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyData {
    ///     thing: String,
    /// }
    ///
    /// let body = MyData {
    ///     thing: "yo".to_string(),
    /// };
    ///
    /// let res = ureq::post("http://httpbin.org/post")
    ///     .send_json(&body)?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    #[cfg(feature = "json")]
    pub fn send_json(self, data: impl serde::ser::Serialize) -> Result<Response<Body>, Error> {
        let request = self.builder.body(())?;
        let body = SendBody::from_json(&data)?;
        do_call(self.agent, request, body)
    }
}

fn do_call(agent: Agent, request: Request<()>, body: SendBody) -> Result<Response<Body>, Error> {
    let response = agent.run_middleware(request, body)?;
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

impl fmt::Debug for RequestBuilder<WithoutBody> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestBuilder<WithoutBody>")
            // unwraps are OK because we can't be in this state without having method+uri
            .field("method", &self.builder.method_ref().unwrap())
            .field("uri", &self.builder.uri_ref().unwrap())
            .finish()
    }
}

impl fmt::Debug for RequestBuilder<WithBody> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestBuilder<WithBody>")
            // unwraps are OK because we can't be in this state without having method+uri
            .field("method", &self.builder.method_ref().unwrap())
            .field("uri", &self.builder.uri_ref().unwrap())
            .finish()
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn disallow_empty_host() {
        let err = crate::get("file:///some/path").call().unwrap_err();
        assert_eq!(err.to_string(), "http: invalid format");
        assert!(matches!(err, Error::Http(_)));
    }

    #[test]
    fn debug_print_without_body() {
        let call = crate::get("https://foo/bar");
        assert_eq!(
            format!("{:?}", call),
            "RequestBuilder<WithoutBody> { method: GET, uri: https://foo/bar }"
        );
    }

    #[test]
    fn debug_print_with_body() {
        let call = crate::post("https://foo/bar");
        assert_eq!(
            format!("{:?}", call),
            "RequestBuilder<WithBody> { method: POST, uri: https://foo/bar }"
        );
    }
}
