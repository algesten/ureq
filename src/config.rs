use std::fmt;
use std::time::Duration;

use hoot::client::flow::RedirectAuthHeaders;

use crate::Proxy;

#[cfg(feature = "_tls")]
use crate::tls::TlsConfig;

/// Config as built by AgentBuilder and then static for the lifetime of the Agent.
///
/// When creating config instances, the `..Default::default()` pattern must be used.
/// See example below.
///
/// # Example
///
/// ```
/// use ureq::AgentConfig;
/// use std::time::Duration;
///
/// let config = AgentConfig {
///     timeout_global: Some(Duration::from_secs(10)),
///     https_only: true,
///     ..Default::default()
/// };
/// ```
#[derive(Clone)]
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

    // This is here to force users of ureq to use the ..Default::default() pattern
    // as part of creating `AgentConfig`. That way we can introduce new settings without
    // it becoming a breaking changes.
    #[doc(hidden)]
    pub _must_use_default: private::Private,
}

// Deliberately not publicly visible.
mod private {
    #[derive(Debug, Clone)]
    pub struct Private;
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

            _must_use_default: private::Private,
        }
    }
}

impl fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentConfig")
            .field("timeout_global", &self.timeout_global)
            .field("timeout_per_call", &self.timeout_per_call)
            .field("timeout_resolve", &self.timeout_resolve)
            .field("timeout_connect", &self.timeout_connect)
            .field("timeout_send_request", &self.timeout_send_request)
            .field("timeout_await_100", &self.timeout_await_100)
            .field("timeout_send_body", &self.timeout_send_body)
            .field("timeout_recv_response", &self.timeout_recv_response)
            .field("timeout_recv_body", &self.timeout_recv_body)
            .field("https_only", &self.https_only)
            .field("no_delay", &self.no_delay)
            .field("max_redirects", &self.max_redirects)
            .field("redirect_auth_headers", &self.redirect_auth_headers)
            .field("user_agent", &self.user_agent)
            .field("input_buffer_size", &self.input_buffer_size)
            .field("output_buffer_size", &self.output_buffer_size)
            .field("max_idle_connections", &self.max_idle_connections)
            .field(
                "max_idle_connections_per_host",
                &self.max_idle_connections_per_host,
            )
            .field("max_idle_age", &self.max_idle_age)
            .field("tls_config", &self.tls_config)
            .field("proxy", &self.proxy)
            .finish()
    }
}