use std::convert::TryFrom;
use std::fmt::Debug;
use std::sync::Arc;

use hoot::client::flow::RedirectAuthHeaders;
use hoot::BodyMode;
use http::{HeaderName, HeaderValue, Method, Request, Response, Uri};

use crate::body::{Body, ResponseInfo};
use crate::pool::{Connection, ConnectionPool};
use crate::proxy::Proxy;
use crate::resolver::{DefaultResolver, Resolver};
use crate::send_body::AsSendBody;
use crate::time::{Duration, Instant};
use crate::transport::{ConnectionDetails, Connector, DefaultConnector, NoBuffers};
use crate::unit::{Event, Input, Unit};
use crate::util::{DebugResponse, HeaderMapExt, UriExt};
use crate::{Error, RequestBuilder, SendBody};
use crate::{WithBody, WithoutBody};

#[cfg(feature = "_tls")]
use crate::tls::TlsConfig;

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
///     .send(b"my password").unwrap();
///
/// let secret = agent
///     .get("http://example.com/get/my-protected-page")
///     .call()
///     .unwrap()
///     .body_mut()
///     .read_to_string(1000)
///     .unwrap();
///
///   println!("Secret is: {}", secret);
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

/// Config as built by AgentBuilder and then static for the lifetime of the Agent.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Timeout for the entire call
    ///
    /// This is end-to-end, from DNS lookup to finishing reading the response body.
    /// Thus it covers all other timeouts.
    ///
    /// Defaults to `None`.
    pub timeout_global: Option<Duration>,

    /// Timeout for call-by-call when following redirects
    ///
    /// This covers a single call and the timeout is reset when
    /// ureq follows a redirections.
    ///
    /// Defaults to `None`.
    pub timeout_per_call: Option<Duration>,

    /// Max duration for doing the DNS lookup when establishing the connection
    ///
    /// Because most platforms do not have an async syscall for looking up
    /// a host name, setting this might force str0m to spawn a thread to handle
    /// the timeout.
    ///
    /// Defaults to `None`.
    pub timeout_resolve: Option<Duration>,

    /// Max duration for establishing the connection
    ///
    /// For a TLS connection this includes opening the socket and doing the TLS handshake.
    ///
    /// Defaults to `None`.
    pub timeout_connect: Option<Duration>,

    /// Max duration for sending the request, but not the request body.
    ///
    /// Defaults to `None`.
    pub timeout_send_request: Option<Duration>,

    /// Max duration for awaiting a 100-continue response.
    ///
    /// Only used if there is a request body and we sent the `Expect: 100-continue`
    /// header to indicate we want the server to respond with 100.
    ///
    /// This defaults to 1 second.
    pub timeout_await_100: Option<Duration>,

    /// Max duration for sending a request body (if there is one)
    ///
    /// Defaults to `None`.
    pub timeout_send_body: Option<Duration>,

    /// Max duration for receiving the response headers, but not the body
    ///
    /// Defaults to `None`.
    pub timeout_recv_response: Option<Duration>,

    /// Max duration for receving the response body.
    ///
    /// Defaults to `None`.
    pub timeout_recv_body: Option<Duration>,

    /// Whether to limit requests (including redirects) to https only
    ///
    /// Defaults to `false`.
    pub https_only: bool,

    /// Disable Nagle's algorithm
    ///
    /// Set TCP_NODELAY. It's up to the transport whether this flag is honored.
    ///
    /// Defaults to `true`.
    pub no_delay: bool,

    /// The max number of redirects to follow before giving up
    ///
    /// Defaults to 10
    pub max_redirects: u32,

    /// How to handle `Authorization` headers when following redirects
    ///
    /// * `Never` (the default) means the authorization header is never attached to a redirected call.
    /// * `SameHost` will keep the header when the redirect is to the same host and under https.
    ///
    /// Defaults to `None`.
    pub redirect_auth_headers: RedirectAuthHeaders,

    /// Value to use for the `User-Agent` field
    ///
    /// Defaults to `ureq <version>`
    pub user_agent: String,

    /// Default size of the input buffer
    ///
    /// The default connectors use this setting.
    ///
    /// Defaults to 512kb.
    pub input_buffer_size: usize,

    /// Default size of the output buffer.
    ///
    /// The default connectors use this setting.
    ///
    /// Defaults to 512kb.
    pub output_buffer_size: usize,

    /// Max number of idle pooled connections overall.
    ///
    /// Defaults to 10
    pub max_idle_connections: usize,

    /// Max number of idle pooled connections per host/port combo.
    ///
    /// Defaults to 3
    pub max_idle_connections_per_host: usize,

    /// Max duration to keep an idle connection in the pool
    ///
    /// Defaults to 15 seconds
    pub max_idle_age: Duration,

    /// Config for TLS.
    ///
    /// This config is generic for all TLS connectors.
    #[cfg(feature = "_tls")]
    pub tls_config: TlsConfig,

    /// Proxy configuration.
    ///
    /// Picked up from environment when using [`AgentConfig::default()`] or
    /// [`Agent::new_with_defaults()`].
    pub proxy: Option<Proxy>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            timeout_global: None,
            timeout_per_call: None,
            timeout_resolve: None,
            timeout_connect: None,
            timeout_send_request: None,
            timeout_await_100: Some(Duration::from_secs(1)),
            timeout_send_body: None,
            timeout_recv_response: None,
            timeout_recv_body: None,
            https_only: false,
            no_delay: true,
            max_redirects: 10,
            redirect_auth_headers: RedirectAuthHeaders::Never,
            user_agent: "ureq".to_string(), // TODO(martin): add version
            input_buffer_size: 128 * 1024,
            output_buffer_size: 128 * 1024,
            max_idle_connections: 10,
            max_idle_connections_per_host: 3,
            max_idle_age: Duration::from_secs(15),

            #[cfg(feature = "_tls")]
            tls_config: TlsConfig::with_native_roots(),

            proxy: Proxy::try_from_env(),
        }
    }
}

impl Agent {
    pub fn new(config: AgentConfig, connector: impl Connector, resolver: impl Resolver) -> Self {
        let pool = Arc::new(ConnectionPool::new(connector, &config));

        Agent {
            config: Arc::new(config),
            pool,
            resolver: Arc::new(resolver),

            #[cfg(feature = "cookies")]
            jar: Arc::new(crate::cookies::SharedCookieJar::new()),
        }
    }

    pub(crate) fn new_with_defaults() -> Self {
        Agent::new(
            AgentConfig::default(),
            DefaultConnector::new(),
            DefaultResolver::default(),
        )
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
    /// agent.get("https://www.google.com/").call().unwrap();
    ///
    /// // Saves (persistent) cookies
    /// let mut file = File::create("cookies.json").unwrap();
    /// agent.cookie_jar().save_json(&mut file).unwrap();
    /// ```
    #[cfg(feature = "cookies")]
    pub fn cookie_jar(&self) -> crate::cookies::CookieJar<'_> {
        self.jar.lock()
    }

    pub fn run(&self, request: Request<impl AsSendBody>) -> Result<Response<Body>, Error> {
        let (parts, mut body) = request.into_parts();
        let body = body.as_body();
        let request = Request::from_parts(parts, ());

        self.do_run(request, body, Instant::now)
    }

    pub(crate) fn do_run(
        &self,
        request: Request<()>,
        body: SendBody,
        current_time: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Result<Response<Body>, Error> {
        let send_body_mode = if request.headers().has_send_body_mode() {
            None
        } else {
            Some(body.body_mode())
        };

        let mut unit = Unit::new(self.config.clone(), current_time(), request, body)?;

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

                    unit.handle_input(current_time(), Input::Begin, &mut [])?;
                }

                Event::Prepare { uri } => {
                    #[cfg(not(feature = "cookies"))]
                    let _ = uri;
                    #[cfg(feature = "cookies")]
                    {
                        let value = self.jar.get_request_cookies(uri);
                        if !value.is_empty() {
                            let value = HeaderValue::from_str(&value).map_err(|_| {
                                Error::Other("Cookie value is an invalid http-header")
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
                        set_header(&mut unit, current_time(), "accept-encoding", value);
                    }

                    if let Some(send_body_mode) = send_body_mode {
                        println!("{:?}", send_body_mode);

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

                    unit.handle_input(current_time(), Input::Prepared, &mut [])?;
                }

                Event::Resolve { uri, timeout } => {
                    // Before resolving the URI we need to ensure it is a full URI. We
                    // cannot make requests with partial uri like "/path".
                    uri.ensure_full_url()?;

                    addr = Some(self.resolver.resolve(uri, timeout)?);
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
                    connection.await_input(timeout)?;
                    let (input, output) = connection.buffers().input_and_output();

                    let input_used =
                        unit.handle_input(current_time(), Input::Data { input }, output)?;

                    connection.consume_input(input_used);
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

        let (parts, _) = response.into_parts();
        let info = ResponseInfo::new(&parts.headers, recv_body_mode);
        let recv_body = Body::new(unit, connection, info, current_time);
        let response = Response::from_parts(parts, recv_body);

        info!("{:?}", DebugResponse(&response));

        Ok(response)
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
