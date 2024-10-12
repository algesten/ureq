use std::convert::TryFrom;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use http::{HeaderName, HeaderValue, Method, Request, Response, Uri, Version};

use crate::body::Body;
use crate::config::{Config, ConfigBuilder, RequestLevelConfig, RequestScope};
use crate::query::{parse_query_params, QueryParam};
use crate::send_body::AsSendBody;
use crate::util::private::Private;
use crate::util::UriExt;
use crate::{Agent, Error, SendBody};

/// Transparent wrapper around [`http::request::Builder`].
///
/// The purpose is to provide the [`.call()`][RequestBuilder::call] and [`.send()`][RequestBuilder::send]
/// functions to make a simpler API for sending requests.
pub struct RequestBuilder<B> {
    agent: Agent,
    builder: http::request::Builder,
    query_extra: Vec<QueryParam<'static>>,

    // This is only used in case http::request::Builder contains an error
    // (such as URL parsing error), and the user wants a `.config()`.
    dummy_config: Option<Box<Config>>,

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

    /// Add a query paramter to the URL.
    ///
    /// Always appends a new parameter, also when using the name of
    /// an already existing one.
    ///
    /// # Examples
    ///
    /// ```
    /// let req = ureq::get("https://httpbin.org/get")
    ///     .query("my_query", "with_value");
    /// ```
    pub fn query<K, V>(mut self, key: K, value: V) -> Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.query_extra
            .push(QueryParam::new_key_value(key.as_ref(), value.as_ref()));
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

    /// Override agent level config on the request level.
    ///
    /// The agent config is copied and modified on request level.
    ///
    /// # Example
    ///
    /// ```
    /// use ureq::Agent;
    ///
    /// let agent: Agent = Agent::config_builder()
    ///     .https_only(false)
    ///     .build()
    ///     .into();
    ///
    /// let request = agent.get("http://httpbin.org/get")
    ///     .config()
    ///     // override agent default for this request
    ///     .https_only(true)
    ///     .build();
    ///
    /// // Make the request
    /// let result = request.call();
    ///
    /// // The https_only was set on request level
    /// assert!(matches!(result.unwrap_err(), ureq::Error::RequireHttpsOnly(_)));
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn config(self) -> ConfigBuilder<RequestScope<Any>> {
        ConfigBuilder(RequestScope(self))
    }

    pub(crate) fn request_level_config(&mut self) -> &mut Config {
        let Some(exts) = self.builder.extensions_mut() else {
            // This means self.builder has an error such as URL parsing error.
            // The error will surface on .call() (or .send()) and we fill in
            // a dummy Config meanwhile.
            return self
                .dummy_config
                .get_or_insert_with(|| Box::new(Config::default()));
        };

        if exts.get::<RequestLevelConfig>().is_none() {
            exts.insert(self.agent.new_request_level_config());
        }

        // Unwrap is OK because of above check
        let req_level: &mut RequestLevelConfig = exts.get_mut().unwrap();

        &mut req_level.0
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
            query_extra: vec![],
            dummy_config: None,
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
        do_call(self.agent, request, self.query_extra, SendBody::none())
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
            query_extra: vec![],
            dummy_config: None,
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
        do_call(self.agent, request, self.query_extra, data_ref.as_body())
    }

    /// Send an empty body.
    ///
    /// The method is POST, PUT or PATCH, which normally has a body. Using
    /// this function makes it explicit you want to send an empty body despite
    /// the method.
    ///
    /// This is equivalent to `.send(&[])`.
    ///
    /// ```
    /// let res = ureq::post("http://httpbin.org/post")
    ///     .send_empty()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn send_empty(self) -> Result<Response<Body>, Error> {
        self.send(&[])
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
        do_call(self.agent, request, self.query_extra, body)
    }
}

fn do_call(
    agent: Agent,
    mut request: Request<()>,
    query_extra: Vec<QueryParam<'static>>,
    body: SendBody,
) -> Result<Response<Body>, Error> {
    if !query_extra.is_empty() {
        request.uri().ensure_valid_url()?;
        request = amend_request_query(request, query_extra.into_iter());
    }
    let response = agent.run_via_middleware(request, body)?;
    Ok(response)
}

fn amend_request_query(
    request: Request<()>,
    query_extra: impl Iterator<Item = QueryParam<'static>>,
) -> Request<()> {
    let (mut parts, body) = request.into_parts();
    let uri = parts.uri;
    let mut path = uri.path().to_string();
    let query_existing = parse_query_params(uri.query().unwrap_or(""));

    let mut do_first = true;

    fn append<'a>(
        path: &mut String,
        do_first: &mut bool,
        iter: impl Iterator<Item = QueryParam<'a>>,
    ) {
        for q in iter {
            if *do_first {
                *do_first = false;
                path.push('?');
            } else {
                path.push('&');
            }
            path.push_str(&q);
        }
    }

    append(&mut path, &mut do_first, query_existing);
    append(&mut path, &mut do_first, query_extra);

    // Unwraps are OK, because we had a correct URI to begin with
    let rebuild = Uri::builder()
        .scheme(uri.scheme().unwrap().clone())
        .authority(uri.authority().unwrap().clone())
        .path_and_query(path)
        .build()
        .unwrap();

    parts.uri = rebuild;

    Request::from_parts(parts, body)
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
    use std::time::Duration;

    use crate::get;
    use crate::test::init_test_log;

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

    #[test]
    fn config_after_broken_url() {
        init_test_log();
        get("http://x.y.z/ borked url")
            .config()
            .timeout_global(Some(Duration::from_millis(1)))
            .build();
    }

    #[test]
    fn add_params_to_request_without_query() {
        let request = Request::builder()
            .uri("https://foo.bar/path")
            .body(())
            .unwrap();

        let amended = amend_request_query(
            request,
            vec![
                QueryParam::new_key_value("x", "z"),
                QueryParam::new_key_value("ab", "cde"),
            ]
            .into_iter(),
        );

        assert_eq!(amended.uri(), "https://foo.bar/path?x=z&ab=cde");
    }

    #[test]
    fn add_params_to_request_with_query() {
        let request = Request::builder()
            .uri("https://foo.bar/path?x=z")
            .body(())
            .unwrap();

        let amended = amend_request_query(
            request,
            vec![QueryParam::new_key_value("ab", "cde")].into_iter(),
        );

        assert_eq!(amended.uri(), "https://foo.bar/path?x=z&ab=cde");
    }

    #[test]
    fn add_params_that_need_percent_encoding() {
        let request = Request::builder()
            .uri("https://foo.bar/path")
            .body(())
            .unwrap();

        let amended = amend_request_query(
            request,
            vec![QueryParam::new_key_value("å ", "i åa ä e ö")].into_iter(),
        );

        assert_eq!(
            amended.uri(),
            "https://foo.bar/path?%C3%A5%20=i%20%C3%A5a%20%C3%A4%20e%20%C3%B6"
        );
    }
}
