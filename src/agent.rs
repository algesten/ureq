use std::convert::TryFrom;
use std::fmt::Debug;
use std::sync::Arc;

use hoot::client::flow::RedirectAuthHeaders;
use http::{Method, Request, Response, Uri};

use crate::body::Body;
use crate::pool::{Connection, ConnectionPool};
use crate::proxy::Proxy;
use crate::resolver::{DefaultResolver, Resolver};
use crate::send_body::AsBody;
use crate::time::{Duration, Instant};
use crate::transport::{ConnectionDetails, Connector, DefaultConnector, NoBuffers};
use crate::unit::{Event, Input, Unit};
use crate::util::UriExt;
use crate::{Error, RequestBuilder, SendBody};

#[cfg(feature = "_tls")]
use crate::tls::TlsConfig;

#[derive(Debug, Clone)]
pub struct Agent {
    config: Arc<AgentConfig>,
    pool: Arc<ConnectionPool>,
    resolver: Arc<dyn Resolver>,
    proxy: Option<Proxy>,

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
    pub timeout_global: Option<Duration>,

    /// Timeout for call-by-call when following redirects
    ///
    /// This covers a single call and the timeout is reset when
    /// ureq follows a redirections.
    pub timeout_per_call: Option<Duration>,

    /// Max duration for doing the DNS lookup when establishing the connection
    ///
    /// Because most platforms do not have an async syscall for looking up
    /// a host name, setting this might force str0m to spawn a thread to handle
    /// the timeout.
    pub timeout_resolve: Option<Duration>,

    /// Max duration for establishing the connection
    ///
    /// For a TLS connection this includes opening the socket and doing the TLS handshake.
    pub timeout_connect: Option<Duration>,

    /// Max duration for sending the request, but not the request body.
    pub timeout_send_request: Option<Duration>,

    /// Max duration for awaiting a 100-continue response.
    ///
    /// Only used if there is a request body and we sent the `Expect: 100-continue`
    /// header to indicate we want the server to respond with 100.
    ///
    /// This defaults to 1 second.
    pub timeout_await_100: Option<Duration>,

    /// Max duration for sending a request body (if there is one)
    pub timeout_send_body: Option<Duration>,

    /// Max duration for receiving the response headers, but not the body
    pub timeout_recv_response: Option<Duration>,

    /// Max duration for receving the response body.
    pub timeout_recv_body: Option<Duration>,

    /// Whether to limit requests (including redirects) to https only
    ///
    /// Defaults to `false`.
    pub https_only: bool,

    /// Disable the nagle algorithm
    ///
    /// TODO(martin): more words here
    pub no_delay: bool,

    /// The max number of redirects to follow before giving up
    ///
    /// Defaults to 10
    pub max_redirects: u32,

    /// How to handle `Authorization` headers when following redirects
    ///
    /// * `Never` (the default) means the authorization header is never attached to a redirected call.
    /// * `SameHost` will keep the header when the redirect is to the same host and under https.
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
        }
    }
}

impl Agent {
    pub fn new(
        config: AgentConfig,
        connector: impl Connector,
        resolver: impl Resolver,
        proxy: Option<Proxy>,
    ) -> Self {
        let pool = Arc::new(ConnectionPool::new(connector, &config));

        Agent {
            config: Arc::new(config),
            pool,
            resolver: Arc::new(resolver),
            proxy,

            #[cfg(feature = "cookies")]
            jar: Arc::new(crate::cookies::SharedCookieJar::new()),
        }
    }

    pub(crate) fn new_default() -> Self {
        Agent::new(
            AgentConfig::default(),
            DefaultConnector::new(),
            DefaultResolver::default(),
            Proxy::try_from_env(),
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
    /// agent.get("https://www.google.com/").call()?;
    ///
    /// // Saves (persistent) cookies
    /// let mut file = File::create("cookies.json")?;
    /// agent.cookie_jar().save_json(&mut file).unwrap();
    /// # Ok::<_, ureq::Error>(())
    /// ```
    #[cfg(feature = "cookies")]
    pub fn cookie_jar(&self) -> crate::cookies::CookieJar<'_> {
        self.jar.lock()
    }

    pub fn run(&self, request: Request<impl AsBody>) -> Result<Response<Body>, Error> {
        let (parts, mut body) = request.into_parts();
        let body = body.as_body();
        let request = Request::from_parts(parts, ());

        self.do_run(request, body, Instant::now)
    }

    // TODO(martin): Can we improve this signature? The ideal would be:
    // fn run(&self, request: Request<impl Body>) -> Result<Response<impl Body>, Error>

    // TODO(martin): One design idea is to be able to create requests in one thread, then
    // actually run them to completion in another. &mut self here makes it impossible to use
    // Agent in such a design. Is that a concern?
    pub(crate) fn do_run(
        &self,
        request: Request<()>,
        body: SendBody,
        current_time: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Result<Response<Body>, Error> {
        let mut unit = Unit::new(self.config.clone(), current_time(), request, body)?;

        let mut addr = None;
        let mut connection: Option<Connection> = None;
        let mut response;
        let mut no_buffers = NoBuffers;

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
                        let input = Input::Header {
                            name: http::HeaderName::from_static("cookie"),
                            value: http::HeaderValue::from_str(&value).map_err(|_| {
                                Error::Other("Cookie value is an invalid http-header")
                            })?,
                        };
                        unit.handle_input(current_time(), input, &mut [])?;
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
                        proxy: &self.proxy,
                        resolver: &*self.resolver,
                        config: &self.config,
                        now: current_time(),
                        timeout,
                    };
                    connection = Some(self.pool.connect(&details)?);

                    unit.handle_input(current_time(), Input::ConnectionOpen, &mut [])?;
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
        let recv_body = Body::new(unit, connection, current_time);
        let response = Response::from_parts(parts, recv_body);

        info!("{}", response.status());

        Ok(response)
    }
}

macro_rules! mk_method {
    ($($f:tt, $m:tt),*) => {
        impl Agent {
            $(
                #[doc = concat!("Make a ", stringify!($m), " request")]
                pub fn $f<T>(&self, uri: T) -> RequestBuilder
                where
                    Uri: TryFrom<T>,
                    <Uri as TryFrom<T>>::Error: Into<http::Error>,
                {
                    RequestBuilder::new(self.clone(), Method::$m, uri)
                }
            )*
        }
    };
}

mk_method!(
    get, GET, post, POST, put, PUT, delete, DELETE, head, HEAD, options, OPTIONS, connect, CONNECT,
    patch, PATCH, trace, TRACE
);
