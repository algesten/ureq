use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use hoot::client::flow::RedirectAuthHeaders;
use http::Uri;

use crate::middleware::MiddlewareChain;
use crate::resolver::IpFamily;
use crate::transport::time::{Instant, NextTimeout};
use crate::{Proxy, TimeoutReason};

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
/// use ureq::{AgentConfig, Timeouts};
/// use std::time::Duration;
///
/// let config = AgentConfig {
///     timeouts: Timeouts {
///         global: Some(Duration::from_secs(10)),
///         ..Default::default()
///     },
///     https_only: true,
///     ..Default::default()
/// };
/// ```
#[derive(Clone)]
pub struct AgentConfig {
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
    /// Picked up from environment when using [`AgentConfig::default()`] or
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
    /// Defaults to `64KB`.
    pub max_response_header_size: usize,

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

    /// Middleware used for this agent.
    ///
    /// Defaults to no middleware.
    pub middleware: MiddlewareChain,

    // This is here to force users of ureq to use the ..Default::default() pattern
    // as part of creating `AgentConfig`. That way we can introduce new settings without
    // it becoming a breaking changes.
    #[doc(hidden)]
    pub _must_use_default: private::Private,
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
    // as part of creating `AgentConfig`. That way we can introduce new settings without
    // it becoming a breaking changes.
    #[doc(hidden)]
    pub _must_use_default: private::Private,
}

// Deliberately not publicly visible.
mod private {
    #[derive(Debug, Clone, Copy)]
    pub struct Private;
}

impl AgentConfig {
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

impl Default for AgentConfig {
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

impl fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbg = f.debug_struct("AgentConfig");

        dbg.field("timeouts", &self.timeouts)
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
            .field("proxy", &self.proxy);

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

#[derive(Debug, Default)]
pub(crate) struct CallTimings {
    pub timeouts: Timeouts,
    pub current_time: CurrentTime,
    pub time_global_start: Option<Instant>,
    pub time_call_start: Option<Instant>,
    pub time_resolve: Option<Instant>,
    pub time_connect: Option<Instant>,
    pub time_send_request: Option<Instant>,
    pub time_send_body: Option<Instant>,
    pub time_await_100: Option<Instant>,
    pub time_recv_response: Option<Instant>,
    pub time_recv_body: Option<Instant>,
}

#[derive(Clone)]
pub(crate) struct CurrentTime(Arc<dyn Fn() -> Instant + Send + Sync + 'static>);

impl CurrentTime {
    pub(crate) fn now(&self) -> Instant {
        self.0()
    }
}

impl CallTimings {
    pub(crate) fn now(&self) -> Instant {
        self.current_time.now()
    }

    pub(crate) fn record_timeout(&mut self, reason: TimeoutReason) {
        match reason {
            TimeoutReason::Global => {
                let now = self.now();
                if self.time_global_start.is_none() {
                    self.time_global_start = Some(now);
                }
                self.time_call_start = Some(now);
            }
            TimeoutReason::Resolver => {
                self.time_resolve = Some(self.now());
            }
            TimeoutReason::OpenConnection => {
                self.time_connect = Some(self.now());
            }
            TimeoutReason::SendRequest => {
                self.time_send_request = Some(self.now());
            }
            TimeoutReason::SendBody => {
                self.time_send_body = Some(self.now());
            }
            TimeoutReason::Await100 => {
                self.time_await_100 = Some(self.now());
            }
            TimeoutReason::RecvResponse => {
                self.time_recv_response = Some(self.now());
            }
            TimeoutReason::RecvBody => {
                self.time_recv_body = Some(self.now());
            }
        }
    }

    pub(crate) fn next_timeout(&self, reason: TimeoutReason) -> NextTimeout {
        // self.time_xxx unwraps() below are OK. If the unwrap fails, we have a state
        // bug where we progressed to a certain state without setting the corresponding time.
        let timeouts = &self.timeouts;

        let expire_at = match reason {
            TimeoutReason::Global => timeouts
                .global
                .map(|t| self.time_global_start.unwrap() + t.into()),
            TimeoutReason::Resolver => timeouts
                .resolve
                .map(|t| self.time_call_start.unwrap() + t.into()),
            TimeoutReason::OpenConnection => timeouts
                .connect
                .map(|t| self.time_resolve.unwrap() + t.into()),
            TimeoutReason::SendRequest => timeouts
                .send_request
                .map(|t| self.time_connect.unwrap() + t.into()),
            TimeoutReason::SendBody => timeouts
                .send_body
                .map(|t| self.time_send_request.unwrap() + t.into()),
            TimeoutReason::Await100 => timeouts
                .await_100
                .map(|t| self.time_send_request.unwrap() + t.into()),
            TimeoutReason::RecvResponse => timeouts.recv_response.map(|t| {
                // The fallback order is important. See state diagram in hoot.
                self.time_send_body
                    .or(self.time_await_100)
                    .or(self.time_send_request)
                    .unwrap()
                    + t.into()
            }),
            TimeoutReason::RecvBody => timeouts
                .recv_body
                .map(|t| self.time_recv_response.unwrap() + t.into()),
        }
        .unwrap_or(Instant::NotHappening);

        let global_at = self.global_timeout();

        let (at, reason) = if global_at < expire_at {
            (global_at, TimeoutReason::Global)
        } else {
            (expire_at, reason)
        };

        let after = at.duration_since(self.now());

        NextTimeout { after, reason }
    }

    fn global_timeout(&self) -> Instant {
        let global_start = self.time_global_start.unwrap();
        let call_start = self.time_call_start.unwrap();

        let global_at = global_start
            + self
                .timeouts
                .global
                .map(|t| t.into())
                .unwrap_or(crate::transport::time::Duration::NotHappening);

        let call_at = call_start
            + self
                .timeouts
                .per_call
                .map(|t| t.into())
                .unwrap_or(crate::transport::time::Duration::NotHappening);

        global_at.min(call_at)
    }

    pub(crate) fn new_call(self) -> CallTimings {
        CallTimings {
            timeouts: self.timeouts,
            time_global_start: self.time_global_start,
            current_time: self.current_time,
            ..Default::default()
        }
    }
}

impl fmt::Debug for CurrentTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CurrentTime").finish()
    }
}

impl Default for CurrentTime {
    fn default() -> Self {
        Self(Arc::new(Instant::now))
    }
}
