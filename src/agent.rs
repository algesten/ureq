use std::convert::TryFrom;
use std::fmt::Debug;
use std::sync::Arc;

use hoot::BodyMode;
use http::uri::Scheme;
use http::{HeaderName, HeaderValue, Method, Request, Response, Uri};

use crate::body::{Body, ResponseInfo};
use crate::middleware::MiddlewareNext;
use crate::pool::{Connection, ConnectionPool};
use crate::resolver::{DefaultResolver, Resolver};
use crate::send_body::AsSendBody;
use crate::transport::time::Instant;
use crate::transport::{ConnectionDetails, Connector, DefaultConnector, NoBuffers};
use crate::unit::{Event, Input, Unit};
use crate::util::{DebugResponse, HeaderMapExt, UriExt};
use crate::{AgentConfig, Error, RequestBuilder, SendBody};
use crate::{WithBody, WithoutBody};

/// Agents keep state between requests.
///
/// By default, no state, such as cookies, is kept between requests.
/// But by creating an agent as entry point for the request, we
/// can keep a state.
///
/// ```no_run
/// let mut agent = ureq::agent();
///
/// agent
///     .post("http://example.com/post/login")
///     .send(b"my password")?;
///
/// let secret = agent
///     .get("http://example.com/get/my-protected-page")
///     .call()?
///     .body_mut()
///     .read_to_string()?;
///
///   println!("Secret is: {}", secret);
/// # Ok::<_, ureq::Error>(())
/// ```
///
/// Agent uses inner `Arc`, so cloning an Agent results in an instance
/// that shares the same underlying connection pool and other state.
#[derive(Debug, Clone)]
pub struct Agent {
    config: Arc<AgentConfig>,
    pool: Arc<ConnectionPool>,
    resolver: Arc<dyn Resolver>,

    #[cfg(feature = "cookies")]
    jar: Arc<crate::cookies::SharedCookieJar>,
}

impl Agent {
    /// Creates an agent with defaults.
    pub fn new_with_defaults() -> Self {
        Self::with_parts(
            AgentConfig::default(),
            DefaultConnector::default(),
            DefaultResolver::default(),
        )
    }

    /// Creates an agent with config.
    pub fn new_with_config(config: AgentConfig) -> Self {
        Self::with_parts(
            config,
            DefaultConnector::default(),
            DefaultResolver::default(),
        )
    }

    /// Creates an agent with a bespoke transport and resolver.
    ///
    /// _This is low level API that isn't for regular use of ureq._
    pub fn with_parts(
        config: AgentConfig,
        connector: impl Connector,
        resolver: impl Resolver,
    ) -> Self {
        let pool = Arc::new(ConnectionPool::new(connector, &config));

        Agent {
            config: Arc::new(config),
            pool,
            resolver: Arc::new(resolver),

            #[cfg(feature = "cookies")]
            jar: Arc::new(crate::cookies::SharedCookieJar::new()),
        }
    }

    /// Access the cookie jar.
    ///
    /// Used to persist and manipulate the cookies.
    ///
    /// ```no_run
    /// use std::io::Write;
    /// use std::fs::File;
    ///
    /// let agent = ureq::agent();
    ///
    /// // Cookies set by www.google.com are stored in agent.
    /// agent.get("https://www.google.com/").call()?;
    ///
    /// // Saves (persistent) cookies
    /// let mut file = File::create("cookies.json")?;
    /// agent.cookie_jar().save_json(&mut file)?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    #[cfg(feature = "cookies")]
    pub fn cookie_jar(&self) -> crate::cookies::CookieJar<'_> {
        self.jar.lock()
    }

    /// Run a [`http::Request<impl AsSendBody>`].
    pub fn run(&self, request: Request<impl AsSendBody>) -> Result<Response<Body>, Error> {
        let (parts, mut body) = request.into_parts();
        let body = body.as_body();
        let request = Request::from_parts(parts, ());

        self.do_run(request, body)
    }

    pub(crate) fn run_middleware(
        &self,
        request: Request<()>,
        body: SendBody,
    ) -> Result<Response<Body>, Error> {
        let (parts, _) = request.into_parts();
        let request = http::Request::from_parts(parts, body);

        let next = MiddlewareNext::new(self);
        next.handle(request)
    }

    pub(crate) fn do_run(
        &self,
        request: Request<()>,
        body: SendBody,
    ) -> Result<Response<Body>, Error> {
        // TODO(martin): use this in tests to try timeouts etc.
        let current_time = Instant::now;

        let headers = request.headers();
        let send_body_mode = if headers.has_send_body_mode() {
            None
        } else {
            Some(body.body_mode())
        };
        #[cfg(any(feature = "gzip", feature = "brotli"))]
        let has_header_accept_enc = headers.has_accept_encoding();
        let has_header_ua = headers.has_user_agent();

        let mut unit = Unit::new(self.config.clone(), current_time(), request, body)?;

        // For CONNECT proxy, this is the address of the proxy server, for
        // all other cases it's the address of the URL being requested.
        let mut addr = None;

        let mut connection: Option<Connection> = None;
        let mut response;
        let mut no_buffers = NoBuffers;
        let mut recv_body_mode = BodyMode::NoBody;

        loop {
            // The buffer is owned by the connection. Before we have an open connection,
            // there are no buffers (and the code below should not need it).
            let buffers = connection
                .as_mut()
                .map(|c| c.buffers())
                .unwrap_or(&mut no_buffers);

            match unit.poll_event(current_time(), buffers)? {
                Event::Reset { must_close } => {
                    addr = None;

                    if let Some(c) = connection.take() {
                        if must_close {
                            c.close();
                        } else {
                            c.reuse(current_time());
                        }
                    }

                    recv_body_mode = BodyMode::NoBody;

                    unit.handle_input(current_time(), Input::Begin, &mut [])?;
                }

                Event::Prepare { uri } => {
                    if self.config.https_only && uri.scheme() != Some(&Scheme::HTTPS) {
                        return Err(Error::AgentRequireHttpsOnly(uri.to_string()));
                    }

                    #[cfg(not(feature = "cookies"))]
                    let _ = uri;
                    #[cfg(feature = "cookies")]
                    {
                        let value = self.jar.get_request_cookies(uri);
                        if !value.is_empty() {
                            let value = HeaderValue::from_str(&value).map_err(|_| {
                                Error::CookieValue("Cookie value is an invalid http-header")
                            })?;
                            set_header(&mut unit, current_time(), "cookie", value);
                        }
                    }

                    #[cfg(any(feature = "gzip", feature = "brotli"))]
                    {
                        use once_cell::sync::Lazy;
                        static ACCEPTS: Lazy<String> = Lazy::new(|| {
                            let mut value = String::with_capacity(10);
                            #[cfg(feature = "gzip")]
                            value.push_str("gzip");
                            #[cfg(all(feature = "gzip", feature = "brotli"))]
                            value.push_str(", ");
                            #[cfg(feature = "brotli")]
                            value.push_str("br");
                            value
                        });
                        // unwrap is ok because above ACCEPTS will produce a valid value
                        let value = HeaderValue::from_str(&ACCEPTS).unwrap();
                        if !has_header_accept_enc {
                            set_header(&mut unit, current_time(), "accept-encoding", value);
                        }
                    }

                    if let Some(send_body_mode) = send_body_mode {
                        match send_body_mode {
                            BodyMode::LengthDelimited(v) => {
                                let value = HeaderValue::from(v);
                                set_header(&mut unit, current_time(), "content-length", value);
                            }
                            BodyMode::Chunked => {
                                let value = HeaderValue::from_static("chunked");
                                set_header(&mut unit, current_time(), "transfer-encoding", value);
                            }
                            _ => {}
                        }
                    }

                    if !has_header_ua && !self.config.user_agent.is_empty() {
                        // unwrap is ok because a user might override the agent, and if they
                        // set bad values, it's not really a big problem.
                        let value = HeaderValue::try_from(&self.config.user_agent).unwrap();
                        set_header(&mut unit, current_time(), "user-agent", value);
                    }

                    unit.handle_input(current_time(), Input::Prepared, &mut [])?;
                }

                Event::Resolve { uri, timeout } => {
                    // If we're using a CONNECT proxy, we need to resolve that hostname.
                    let maybe_connect_uri = self.config.connect_proxy_uri();

                    let effective_uri = maybe_connect_uri.unwrap_or(uri);

                    // Before resolving the URI we need to ensure it is a full URI. We
                    // cannot make requests with partial uri like "/path".
                    effective_uri.ensure_valid_url()?;

                    addr = Some(self.resolver.resolve(effective_uri, timeout)?);
                    unit.handle_input(current_time(), Input::Resolved, &mut [])?;
                }

                Event::OpenConnection { uri, timeout } => {
                    let addr = addr.expect("addr to be available after Event::Resolve");

                    let details = ConnectionDetails {
                        uri,
                        addr,
                        resolver: &*self.resolver,
                        config: &self.config,
                        now: current_time(),
                        timeout,
                    };
                    connection = Some(self.pool.connect(&details)?);

                    unit.handle_input(current_time(), Input::ConnectionOpen, &mut [])?;

                    if log_enabled!(log::Level::Info) {
                        let fake_request = unit
                            .fake_request()
                            .expect("fake_request after Input::Prepared");
                        info!("{:?}", fake_request);
                    }
                }

                Event::Await100 { timeout } => {
                    let connection = connection.as_mut().expect("connection for AwaitInput");

                    match connection.await_input(timeout) {
                        Ok(_) => {
                            let input = connection.buffers().input();
                            unit.handle_input(current_time(), Input::Data { input }, &mut [])?
                        }

                        // If we get a timeout while waiting for input, that is not an error,
                        // EndAwait100 progresses the state machine to start reading a response.
                        Err(Error::Timeout(_)) => {
                            unit.handle_input(current_time(), Input::EndAwait100, &mut [])?
                        }
                        Err(e) => return Err(e),
                    };
                }

                Event::Transmit { amount, timeout } => {
                    let connection = connection.as_mut().expect("connection for Transmit");
                    connection.transmit_output(amount, timeout)?;
                }

                Event::AwaitInput { timeout } => {
                    let connection = connection.as_mut().expect("connection for AwaitInput");

                    let has_buffered_input = connection.buffers().can_use_input();

                    if has_buffered_input {
                        // Try use buffered input
                        let (input, output) = connection.buffers().input_and_output();
                        let input_used =
                            unit.handle_input(current_time(), Input::Data { input }, output)?;

                        // If we don't use buffered input, we need to read more.
                        if input_used > 0 {
                            connection.consume_input(input_used);
                            continue;
                        }
                    }

                    loop {
                        // Read more input
                        let made_progress = connection.await_input(timeout)?;
                        let (input, output) = connection.buffers().input_and_output();

                        let input_used =
                            unit.handle_input(current_time(), Input::Data { input }, output)?;
                        connection.consume_input(input_used);

                        if input_used > 0 {
                            break;
                        } else if made_progress {
                            continue;
                        } else {
                            return Err(Error::BodyStalled);
                        }
                    }
                }

                Event::Response { response: r, end } => {
                    response = Some(r);

                    if let Some(b) = unit.body_mode() {
                        recv_body_mode = b;
                    }

                    // end true means one of two things:
                    // 1. This is a non-redirect response
                    // 2. This is a redirect response, and we are not following (any more) redirects
                    if end {
                        break;
                    }
                }

                Event::ResponseBody { .. } => {
                    // Implicitly, if we find ourselves here, we are following a redirect and need
                    // to consume the body to be able to make the next request.
                }
            }
        }

        let response = response.expect("above loop to exit when there is a response");
        let connection = connection.expect("connection to be open");
        let unit = unit.release_body();
        let status = response.status();
        let is_err = status.is_client_error() || status.is_server_error();

        if self.config.http_status_as_error && is_err {
            return Err(Error::StatusCode(status.as_u16()));
        }

        let (parts, _) = response.into_parts();
        let info = ResponseInfo::new(&parts.headers, recv_body_mode);
        let recv_body = Body::new(unit, connection, info, current_time);
        let response = Response::from_parts(parts, recv_body);

        info!("{:?}", DebugResponse(&response));
        trace!("Receive body mode is: {:?}", recv_body_mode);

        Ok(response)
    }

    pub(crate) fn config(&self) -> &AgentConfig {
        &self.config
    }
}

fn set_header(unit: &mut Unit<SendBody>, now: Instant, name: &'static str, value: HeaderValue) {
    let name = HeaderName::from_static(name);
    let input = Input::Header { name, value };
    unit.handle_input(now, input, &mut [])
        .expect("to set header");
}

macro_rules! mk_method {
    ($(($f:tt, $m:tt, $b:ty)),*) => {
        impl Agent {
            $(
                #[doc = concat!("Make a ", stringify!($m), " request using this agent.")]
                #[must_use]
                pub fn $f<T>(&self, uri: T) -> RequestBuilder<$b>
                where
                    Uri: TryFrom<T>,
                    <Uri as TryFrom<T>>::Error: Into<http::Error>,
                {
                    RequestBuilder::<$b>::new(self.clone(), Method::$m, uri)
                }
            )*
        }
    };
}

mk_method!(
    (get, GET, WithoutBody),
    (post, POST, WithBody),
    (put, PUT, WithBody),
    (delete, DELETE, WithoutBody),
    (head, HEAD, WithoutBody),
    (options, OPTIONS, WithoutBody),
    (connect, CONNECT, WithoutBody),
    (patch, PATCH, WithBody),
    (trace, TRACE, WithoutBody)
);

impl From<AgentConfig> for Agent {
    fn from(value: AgentConfig) -> Self {
        Agent::new_with_config(value)
    }
}

#[cfg(test)]
impl crate::Agent {
    pub fn pool_count(&self) -> usize {
        self.pool.pool_count()
    }
}
