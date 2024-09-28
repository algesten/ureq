use std::fmt;
use std::time::Duration;

use hoot::client::flow::RedirectAuthHeaders;
use http::Uri;

use crate::middleware::MiddlewareChain;
use crate::resolver::IpFamily;
use crate::Proxy;

#[cfg(feature = "_tls")]
use crate::tls::TlsConfig;

/// Config primarily for the [`Agent`][crate::Agent], but also per-request.
///
/// # Agent level config
///
/// When creating config instances, the prefered way is to use the `..Default::default()` pattern.
///
/// ## Example
///
/// ```
/// use ureq::{Agent, Config, Timeouts};
/// use std::time::Duration;
///
/// let config = Config {
///     timeouts: Timeouts {
///         global: Some(Duration::from_secs(10)),
///         ..Default::default()
///     },
///     https_only: true,
///     ..Default::default()
/// };
///
/// let agent = Agent::new_with_config(config);
/// ```
///
/// And alternative way is to set properties on an already created config
///
/// ```
/// use ureq::{Agent, Config};
/// use std::time::Duration;
///
/// let mut config = Config::new();
/// config.timeouts.global = Some(Duration::from_secs(10));
/// config.https_only = true;
///
/// let agent: Agent = config.into();
/// ```
///
/// # Request level config
///
/// The config can also be change per-request. Since every request ultimately executes
/// using an [`Agent`][crate::Agent] (also the root-level `ureq::get(...)` have an implicit agent),
/// a request level config clones the agent level config.
///
/// There are two ways of getting a request level config.
///
/// ## Request builder example
///
/// The first way is via [`RequestBuilder::config()`][crate::RequestBuilder::config].
///
/// ```
/// use ureq::{Agent, Config};
///
/// let agent: Agent = Config {
///     https_only: false,
///     ..Default::default()
/// }.into();
///
/// let mut builder = agent.get("http://httpbin.org/get");
///
/// let config = builder.config();
/// config.https_only = true;
/// ```
///
/// ## HTTP request example
///
/// The second way is via [`Agent::configure_request()`][crate::Agent::configure_request].
/// This is used when working with the http crate [`http::Request`] type directly.
///
/// ```
/// use ureq::{Agent, Config};
///
/// let agent: Agent = Config {
///     https_only: false,
///     ..Default::default()
/// }.into();
///
/// let mut request = http::Request::get("http://httpbin.org/get")
///     .body(()).unwrap();
///
/// let config = agent.configure_request(&mut request);
/// config.https_only = true;
/// ```
///
/// # Correct usage
///
/// Note: For a struct with pub fields, Rust dosn't have a way to force the use of
/// `..Default::default()`. `Config` must be instantiated in one two ways:
///
/// 1. `Config::default()` or `Config::new()`.
/// 2. `Config { <override defaults>, ..Default::default() }`
///
/// Any other way to construct the config is not valid, and breaking changes arising
/// from doing that are not considered breaking. Specifically it is not correct to use
/// `Config { ... }` without a `..Default::default()`.
///
#[derive(Clone)]
pub struct Config {
    /// Whether to treat 4xx and 5xx HTTP status codes as
    /// [`Err(Error::StatusCode))`](crate::Error::StatusCode).
    ///
    /// Defaults to `true`.
    pub http_status_as_error: bool,

    /// Whether to limit requests (including redirects) to https only
    ///
    /// Defaults to `false`.
    pub https_only: bool,

    /// Configuration of IPv4/IPv6.
    ///
    /// This affects the resolver.
    ///
    /// Defaults to `IpFamily::Any`.
    pub ip_family: IpFamily,

    /// Config for TLS.
    ///
    /// This config is generic for all TLS connectors.
    #[cfg(feature = "_tls")]
    pub tls_config: TlsConfig,

    /// Proxy configuration.
    ///
    /// Picked up from environment when using [`Config::default()`] or
    /// [`Agent::new_with_defaults()`][crate::Agent::new_with_defaults].
    pub proxy: Option<Proxy>,

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
    /// Defaults to `ureq/<version>`
    pub user_agent: String,

    /// The timeout settings on agent level.
    ///
    /// This can be overridden per request.
    pub timeouts: Timeouts,

    /// Max size of the HTTP response header.
    ///
    /// From the status, including all headers up until the body.
    ///
    /// Defaults to 64kb.
    pub max_response_header_size: usize,

    /// Default size of the input buffer
    ///
    /// The default connectors use this setting.
    ///
    /// Defaults to 128kb.
    pub input_buffer_size: usize,

    /// Default size of the output buffer.
    ///
    /// The default connectors use this setting.
    ///
    /// Defaults to 128kb.
    pub output_buffer_size: usize,

    /// Max number of idle pooled connections overall.
    ///
    /// This setting has no effect when used per-request.
    ///
    /// Defaults to 10
    pub max_idle_connections: usize,

    /// Max number of idle pooled connections per host/port combo.
    ///
    /// This setting has no effect when used per-request.
    ///
    /// Defaults to 3
    pub max_idle_connections_per_host: usize,

    /// Max duration to keep an idle connection in the pool
    ///
    /// This can also be configured per-request to be shorter than the pool.
    /// For example: if the pool is configured to 15 seconds and we have a
    /// connection with an age of 10 seconds, a request setting this config
    /// property to 3 seconds, would ignore the pooled connection (but still
    /// leave it in the pool).
    ///
    /// Defaults to 15 seconds
    pub max_idle_age: Duration,

    /// Middleware used for this agent.
    ///
    /// Defaults to no middleware.
    pub middleware: MiddlewareChain,

    // This is here to force users of ureq to use the ..Default::default() pattern
    // as part of creating `Config`. That way we can introduce new settings without
    // it becoming a breaking changes.
    #[doc(hidden)]
    pub _must_use_default: private::Private,
}

impl Config {
    /// Creates a new Config with defaults values.
    ///
    /// This is the same as `Config::default()`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Request timeout configuration.
///
/// This can be configured both on Agent level as well as per request.
#[derive(Clone, Copy)]
pub struct Timeouts {
    /// Timeout for the entire call
    ///
    /// This is end-to-end, from DNS lookup to finishing reading the response body.
    /// Thus it covers all other timeouts.
    ///
    /// Defaults to `None`.
    pub global: Option<Duration>,

    /// Timeout for call-by-call when following redirects
    ///
    /// This covers a single call and the timeout is reset when
    /// ureq follows a redirections.
    ///
    /// Defaults to `None`.
    pub per_call: Option<Duration>,

    /// Max duration for doing the DNS lookup when establishing the connection
    ///
    /// Because most platforms do not have an async syscall for looking up
    /// a host name, setting this might force str0m to spawn a thread to handle
    /// the timeout.
    ///
    /// Defaults to `None`.
    pub resolve: Option<Duration>,

    /// Max duration for establishing the connection
    ///
    /// For a TLS connection this includes opening the socket and doing the TLS handshake.
    ///
    /// Defaults to `None`.
    pub connect: Option<Duration>,

    /// Max duration for sending the request, but not the request body.
    ///
    /// Defaults to `None`.
    pub send_request: Option<Duration>,

    /// Max duration for awaiting a 100-continue response.
    ///
    /// Only used if there is a request body and we sent the `Expect: 100-continue`
    /// header to indicate we want the server to respond with 100.
    ///
    /// This defaults to 1 second.
    pub await_100: Option<Duration>,

    /// Max duration for sending a request body (if there is one)
    ///
    /// Defaults to `None`.
    pub send_body: Option<Duration>,

    /// Max duration for receiving the response headers, but not the body
    ///
    /// Defaults to `None`.
    pub recv_response: Option<Duration>,

    /// Max duration for receving the response body.
    ///
    /// Defaults to `None`.
    pub recv_body: Option<Duration>,

    // This is here to force users of ureq to use the ..Default::default() pattern
    // as part of creating `Config`. That way we can introduce new settings without
    // it becoming a breaking changes.
    #[doc(hidden)]
    pub _must_use_default: private::Private,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestLevelConfig(pub Config);

// Deliberately not publicly visible.
mod private {
    #[derive(Debug, Clone, Copy)]
    pub struct Private;
}

impl Config {
    pub(crate) fn connect_proxy_uri(&self) -> Option<&Uri> {
        let proxy = self.proxy.as_ref()?;

        if !proxy.proto().is_connect() {
            return None;
        }

        Some(proxy.uri())
    }
}

pub static DEFAULT_USER_AGENT: &str =
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

impl Default for Config {
    fn default() -> Self {
        Self {
            http_status_as_error: true,
            https_only: false,
            ip_family: IpFamily::Any,
            #[cfg(feature = "_tls")]
            tls_config: TlsConfig::default(),
            proxy: Proxy::try_from_env(),
            no_delay: true,
            max_redirects: 10,
            redirect_auth_headers: RedirectAuthHeaders::Never,
            user_agent: DEFAULT_USER_AGENT.to_string(),
            timeouts: Timeouts::default(),
            max_response_header_size: 64 * 1024,
            input_buffer_size: 128 * 1024,
            output_buffer_size: 128 * 1024,
            max_idle_connections: 10,
            max_idle_connections_per_host: 3,
            max_idle_age: Duration::from_secs(15),
            middleware: MiddlewareChain::default(),

            _must_use_default: private::Private,
        }
    }
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            global: None,
            per_call: None,
            resolve: None,
            connect: None,
            send_request: None,
            await_100: Some(Duration::from_secs(1)),
            send_body: None,
            recv_response: None,
            recv_body: None,

            _must_use_default: private::Private,
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbg = f.debug_struct("Config");

        dbg.field("http_status_as_error", &self.http_status_as_error)
            .field("https_only", &self.https_only)
            .field("ip_family", &self.ip_family)
            .field("tls_config", &self.tls_config)
            .field("proxy", &self.proxy)
            .field("no_delay", &self.no_delay)
            .field("max_redirects", &self.max_redirects)
            .field("redirect_auth_headers", &self.redirect_auth_headers)
            .field("user_agent", &self.user_agent)
            .field("timeouts", &self.timeouts)
            .field("max_response_header_size", &self.max_response_header_size)
            .field("input_buffer_size", &self.input_buffer_size)
            .field("output_buffer_size", &self.output_buffer_size)
            .field("max_idle_connections", &self.max_idle_connections)
            .field(
                "max_idle_connections_per_host",
                &self.max_idle_connections_per_host,
            )
            .field("max_idle_age", &self.max_idle_age)
            .field("middleware", &self.middleware);

        #[cfg(feature = "_tls")]
        {
            dbg.field("tls_config", &self.tls_config);
        }

        dbg.finish()
    }
}

impl fmt::Debug for Timeouts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Timeouts")
            .field("global", &self.global)
            .field("per_call", &self.per_call)
            .field("resolve", &self.resolve)
            .field("connect", &self.connect)
            .field("send_request", &self.send_request)
            .field("await_100", &self.await_100)
            .field("send_body", &self.send_body)
            .field("recv_response", &self.recv_response)
            .field("recv_body", &self.recv_body)
            .finish()
    }
}
