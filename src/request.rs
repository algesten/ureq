use std::convert::TryFrom;
use std::fmt;
use std::marker::PhantomData;

use http::{Extensions, HeaderMap, HeaderName, HeaderValue};
use http::{Method, Request, Response, Uri, Version};

use crate::body::Body;
use crate::config::typestate::RequestScope;
use crate::config::{Config, ConfigBuilder, RequestLevelConfig};
use crate::http;
use crate::query::form_url_enc;
use crate::query::{parse_query_params, QueryParam};
use crate::send_body::AsSendBody;
use crate::util::private::Private;
use crate::util::HeaderMapExt;
use crate::util::UriExt;
use crate::{Agent, Error, SendBody};

/// Transparent wrapper around [`http::request::Builder`].
///
/// The purpose is to provide the [`.call()`][RequestBuilder::call] and [`.send()`][RequestBuilder::send]
/// and additional helpers for query parameters like [`.query()`][RequestBuilder::query] functions to
/// make an API for sending requests.
pub struct RequestBuilder<B> {
    agent: Agent,
    builder: http::request::Builder,
    query_extra: Vec<QueryParam<'static>>,

    // This is only used in case http::request::Builder contains an error
    // (such as URL parsing error), and the user wants a `.config()`.
    dummy_config: Option<Box<Config>>,

    _ph: PhantomData<B>,
}

/// Typestate when [`RequestBuilder`] has no send body.
///
/// `RequestBuilder<WithoutBody>`
///
/// Methods: GET, DELETE, HEAD, OPTIONS, CONNECT, TRACE
#[derive(Debug)]
pub struct WithoutBody(());
impl Private for WithoutBody {}

/// Typestate when [`RequestBuilder`] needs to a send body.
///
/// `RequestBuilder<WithBody>`
///
/// Methods: POST, PUT, PATCH
#[derive(Debug)]
pub struct WithBody(());
impl Private for WithBody {}

impl<Any> RequestBuilder<Any> {
    /// Get the HTTP Method for this request.
    ///
    /// By default this is `GET`. If builder has error, returns None.
    ///
    /// # Examples
    ///
    /// ```
    /// use ureq::http::Method;
    ///
    /// let req = ureq::get("http://httpbin.org/get");
    /// assert_eq!(req.method_ref(),Some(&Method::GET));
    ///
    /// let req = ureq::post("http://httpbin.org/post");
    /// assert_eq!(req.method_ref(),Some(&Method::POST));
    /// ```
    pub fn method_ref(&self) -> Option<&Method> {
        self.builder.method_ref()
    }

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

    /// Get header on this request builder.
    ///
    /// When builder has error returns `None`.
    ///
    /// # Example
    ///
    /// ```
    /// let req = ureq::get("http://httpbin.org/get")
    ///     .header("Accept", "text/html")
    ///     .header("X-Custom-Foo", "bar");
    /// let headers = req.headers_ref().unwrap();
    /// assert_eq!( headers["Accept"], "text/html" );
    /// assert_eq!( headers["X-Custom-Foo"], "bar" );
    /// ```
    pub fn headers_ref(&self) -> Option<&HeaderMap<HeaderValue>> {
        self.builder.headers_ref()
    }

    /// Get headers on this request builder.
    ///
    /// When builder has error returns `None`.
    ///
    /// # Example
    ///
    /// ```
    /// # use ureq::http::header::HeaderValue;
    /// let mut req =  ureq::get("http://httpbin.org/get");
    /// {
    ///   let headers = req.headers_mut().unwrap();
    ///   headers.insert("Accept", HeaderValue::from_static("text/html"));
    ///   headers.insert("X-Custom-Foo", HeaderValue::from_static("bar"));
    /// }
    /// let headers = req.headers_ref().unwrap();
    /// assert_eq!( headers["Accept"], "text/html" );
    /// assert_eq!( headers["X-Custom-Foo"], "bar" );
    /// ```
    pub fn headers_mut(&mut self) -> Option<&mut HeaderMap<HeaderValue>> {
        self.builder.headers_mut()
    }

    /// Add a query parameter to the URL.
    ///
    /// Always appends a new parameter, also when using the name of
    /// an already existing one. Both key and value are percent-encoded
    /// according to the URL specification.
    ///
    /// # Examples
    ///
    /// ```
    /// // Creates a URL with an encoded query parameter:
    /// // https://httpbin.org/get?my_query=with%20value
    /// let req = ureq::get("https://httpbin.org/get")
    ///     .query("my_query", "with value");
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

    /// Add a query parameter to the URL without percent-encoding.
    ///
    /// Always appends a new parameter, also when using the name of
    /// an already existing one. Neither key nor value are percent-encoded,
    /// which allows you to use pre-encoded values or bypass encoding.
    ///
    /// **Important note**: When using this method, you must ensure that your
    /// query parameters don't contain characters that would make the URI invalid,
    /// such as spaces or control characters. You are responsible for any pre-encoding
    /// needed for URI validity. If you're unsure, use the regular `query()` method instead.
    ///
    /// # Examples
    ///
    /// ```
    /// // Creates a URL with a raw query parameter:
    /// // https://httpbin.org/get?my_query=pre-encoded%20value
    /// let req = ureq::get("https://httpbin.org/get")
    ///     .query_raw("my_query", "pre-encoded%20value");
    /// ```
    pub fn query_raw<K, V>(mut self, key: K, value: V) -> Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.query_extra
            .push(QueryParam::new_key_value_raw(key.as_ref(), value.as_ref()));
        self
    }

    /// Set multi query parameters.
    ///
    /// Both keys and values are percent-encoded according to the URL specification.
    ///
    /// For example, to set `?format=json&dest=%2Flogin`
    ///
    /// ```
    /// let query = vec![
    ///     ("format", "json"),
    ///     ("dest", "/login"),
    /// ];
    ///
    /// let response = ureq::get("http://httpbin.org/get")
    ///    .query_pairs(query)
    ///    .call()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn query_pairs<I, K, V>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.query_extra.extend(
            iter.into_iter()
                .map(|(k, v)| QueryParam::new_key_value(k.as_ref(), v.as_ref())),
        );
        self
    }
    /// Set multi query parameters without percent-encoding.
    ///
    /// Neither keys nor values are percent-encoded, which allows you to use
    /// pre-encoded values or bypass encoding.
    ///
    /// **Important note**: When using this method, you must ensure that your
    /// query parameters don't contain characters that would make the URI invalid,
    /// such as spaces or control characters. You are responsible for any pre-encoding
    /// needed for URI validity. If you're unsure, use the regular `query_pairs()` method instead.
    ///
    /// For example, to set `?format=json&dest=/login` without encoding:
    ///
    /// ```
    /// let query = vec![
    ///     ("format", "json"),
    ///     ("dest", "/login"),
    /// ];
    ///
    /// let response = ureq::get("http://httpbin.org/get")
    ///    .query_pairs_raw(query)
    ///    .call()?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn query_pairs_raw<I, K, V>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.query_extra.extend(
            iter.into_iter()
                .map(|(k, v)| QueryParam::new_key_value_raw(k.as_ref(), v.as_ref())),
        );
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

    /// Get the URI for this request
    ///
    /// By default this is `/`.
    ///
    /// # Examples
    ///
    /// ```
    /// let req = ureq::get("http://httpbin.org/get");
    /// assert_eq!(req.uri_ref().unwrap(), "http://httpbin.org/get");
    /// ```
    pub fn uri_ref(&self) -> Option<&Uri> {
        self.builder.uri_ref()
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

    /// Get the HTTP version for this request
    ///
    /// By default this is HTTP/1.1.
    ///
    /// # Examples
    ///
    /// ```
    /// use ureq::http::Version;
    ///
    /// let req = ureq::get("http://httpbin.org/get");
    /// assert_eq!(req.version_ref().unwrap(), &Version::HTTP_11);
    /// ```
    pub fn version_ref(&self) -> Option<&Version> {
        self.builder.version_ref()
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

    /// Adds an extension to this builder
    ///
    /// # Examples
    ///
    /// ```
    /// let req = ureq::get("http://httpbin.org/get")
    ///     .extension("My Extension");
    ///
    /// assert_eq!(req.extensions_ref().unwrap().get::<&'static str>(),
    ///            Some(&"My Extension"));
    /// ```
    pub fn extension<T>(mut self, extension: T) -> Self
    where
        T: Clone + std::any::Any + Send + Sync + 'static,
    {
        self.builder = self.builder.extension(extension);
        self
    }

    /// Get a reference to the extensions for this request builder.
    ///
    /// If the builder has an error, this returns `None`.
    ///
    /// # Example
    ///
    /// ```
    /// let req = ureq::get("http://httpbin.org/get")
    ///     .extension("My Extension").extension(5u32);
    /// let extensions = req.extensions_ref().unwrap();
    /// assert_eq!(extensions.get::<&'static str>(), Some(&"My Extension"));
    /// assert_eq!(extensions.get::<u32>(), Some(&5u32));
    /// ```
    pub fn extensions_ref(&self) -> Option<&Extensions> {
        self.builder.extensions_ref()
    }

    /// Get a mutable reference to the extensions for this request builder.
    ///
    /// If the builder has an error, this returns `None`.
    ///
    /// # Example
    ///
    /// ```
    /// let mut req = ureq::get("http://httpbin.org/get");
    /// let mut extensions = req.extensions_mut().unwrap();
    /// extensions.insert(5u32);
    /// assert_eq!(extensions.get::<u32>(), Some(&5u32));
    /// ```
    pub fn extensions_mut(&mut self) -> Option<&mut Extensions> {
        self.builder.extensions_mut()
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

    /// Force sending a body.
    ///
    /// This is an escape hatch to interact with broken services.
    ///
    /// According to the spec, methods such as GET, DELETE and TRACE should
    /// not have a body. Despite that there are broken API services and
    /// servers that use it.
    ///
    /// Example using DELETE while sending a body.
    ///
    /// ```
    /// let res = ureq::delete("http://httpbin.org/delete")
    ///     // this "unlocks" send() below
    ///     .force_send_body()
    ///     .send("DELETE with body is not correct")?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    pub fn force_send_body(mut self) -> RequestBuilder<WithBody> {
        if let Some(exts) = self.extensions_mut() {
            exts.insert(ForceSendBody);
        }

        RequestBuilder {
            agent: self.agent,
            builder: self.builder,
            query_extra: self.query_extra,
            dummy_config: None,
            _ph: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ForceSendBody;

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
        let mut request = self.builder.body(())?;
        let mut data_ref = data;
        let mut body = data_ref.as_body();

        // Automatically set Content-Type if the body provides one and no Content-Type is already set
        if let Some(content_type) = body.take_content_type() {
            if !request.headers().has_content_type() {
                request
                    .headers_mut()
                    .append(http::header::CONTENT_TYPE, content_type);
            }
        }

        do_call(self.agent, request, self.query_extra, body)
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

    /// Send form encoded data.
    ///
    /// Constructs a [form submission] with the content-type header
    /// `application/x-www-form-urlencoded`. Keys and values will be URL encoded.
    ///
    /// ```
    /// let form = [
    ///     ("name", "martin"),
    ///     ("favorite_bird", "blue-footed booby"),
    /// ];
    ///
    /// let response = ureq::post("http://httpbin.org/post")
    ///    .send_form(form)?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    ///
    /// [form submission]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Methods/POST#url-encoded_form_submission
    pub fn send_form<I, K, V>(self, iter: I) -> Result<Response<Body>, Error>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let iter = iter.into_iter();

        // TODO(martin): can we calculate a size hint for capacity here?
        let mut body = String::new();

        for (k, v) in iter {
            if !body.is_empty() {
                body.push('&');
            }
            body.push_str(&form_url_enc(k.as_ref()));
            body.push('=');
            body.push_str(&form_url_enc(v.as_ref()));
        }

        let body = body.as_body();
        let body = body.with_content_type(HeaderValue::from_static(
            "application/x-www-form-urlencoded",
        ));

        self.send(body)
    }

    /// Send body data as JSON.
    ///
    /// Requires the **json** feature.
    ///
    /// The data typically derives [`Serialize`](serde::Serialize) and is converted
    /// to a string before sending (does allocate). Will set the content-type header
    /// `application/json`.
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
        let body = SendBody::from_json(&data)?;
        self.send(body)
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
    fn query_with_encoding() {
        let request = Request::builder()
            .uri("https://foo.bar/path")
            .body(())
            .unwrap();

        // Test that single quotes and spaces are encoded
        let amended = amend_request_query(
            request,
            vec![QueryParam::new_key_value("key", "value with 'quotes'")].into_iter(),
        );

        assert_eq!(
            amended.uri(),
            "https://foo.bar/path?key=value%20with%20%27quotes%27"
        );
    }

    #[test]
    fn query_raw_without_encoding() {
        let request = Request::builder()
            .uri("https://foo.bar/path")
            .body(())
            .unwrap();

        // Test that raw values remain unencoded (using URI-valid characters)
        let amended = amend_request_query(
            request,
            vec![QueryParam::new_key_value_raw("key", "value-with-'quotes'")].into_iter(),
        );

        assert_eq!(
            amended.uri(),
            "https://foo.bar/path?key=value-with-'quotes'"
        );
    }
    #[test]
    fn encoded_and_raw_combined() {
        let request = Request::builder()
            .uri("https://foo.bar/path")
            .body(())
            .unwrap();

        // Test combination of encoded and unencoded parameters
        let amended = amend_request_query(
            request,
            vec![
                QueryParam::new_key_value("encoded", "value with spaces"),
                QueryParam::new_key_value_raw("raw", "value-without-spaces"),
            ]
            .into_iter(),
        );

        assert_eq!(
            amended.uri(),
            "https://foo.bar/path?encoded=value%20with%20spaces&raw=value-without-spaces"
        );
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
