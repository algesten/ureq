use std::sync::Arc;

use crate::header::{self, Header};
use crate::pool::ConnectionPool;
use crate::proxy::Proxy;
use crate::request::Request;
use crate::resolve::{ArcResolver, StdResolver};
use std::time::Duration;

#[cfg(feature = "cookies")]
use {crate::cookies::CookieTin, cookie::Cookie, cookie_store::CookieStore, url::Url};

#[derive(Debug)]
pub struct AgentBuilder {
    headers: Vec<Header>,
    config: AgentConfig,
    /// Cookies saved between requests.
    /// Invariant: All cookies must have a nonempty domain and path.
    #[cfg(feature = "cookies")]
    cookie_store: Option<CookieStore>,
    resolver: ArcResolver,
}

/// Config as built by AgentBuilder and then static for the lifetime of the Agent.
#[derive(Debug, Clone)]
pub(crate) struct AgentConfig {
    pub max_idle_connections: usize,
    pub max_idle_connections_per_host: usize,
    pub proxy: Option<Proxy>,
    pub timeout_connect: Option<Duration>,
    pub timeout_read: Option<Duration>,
    pub timeout_write: Option<Duration>,
    pub timeout: Option<Duration>,
    pub redirects: u32,
    #[cfg(feature = "tls")]
    pub tls_config: Option<TLSClientConfig>,
}

/// Agents keep state between requests.
///
/// By default, no state, such as cookies, is kept between requests.
/// But by creating an agent as entry point for the request, we
/// can keep a state.
///
/// ```
/// let mut agent = ureq::agent();
///
/// agent.set("x-my-secret-header", "very secret");
///
/// let auth = agent
///     .post("/login")
///     .call(); // blocks.
///
/// if auth.is_err() {
///     println!("Noes!");
/// }
///
/// let secret = agent
///     .get("/my-protected-page")
///     .call(); // blocks and waits for request.
///
/// if secret.is_err() {
///     println!("Wot?!");
/// } else {
///   println!("Secret is: {}", secret.unwrap().into_string().unwrap());
/// }
/// ```
///
/// Agent uses an inner Arc, so cloning an Agent results in an instance
/// that shares the same underlying connection pool and other state.
#[derive(Debug, Clone)]
pub struct Agent {
    pub(crate) config: Arc<AgentConfig>,
    /// Copied into each request of this agent.
    pub(crate) headers: Vec<Header>,
    /// Reused agent state for repeated requests from this agent.
    pub(crate) state: Arc<AgentState>,
}

/// Container of the state
///
/// *Internal API*.
#[derive(Debug)]
pub(crate) struct AgentState {
    /// Reused connections between requests.
    pub(crate) pool: ConnectionPool,
    pub(crate) proxy: Option<Proxy>,
    /// Cookies saved between requests.
    /// Invariant: All cookies must have a nonempty domain and path.
    #[cfg(feature = "cookies")]
    pub(crate) cookie_tin: CookieTin,
    pub(crate) resolver: ArcResolver,
}

impl Agent {
    /// Creates an Agent with default settings.
    ///
    /// Same as `AgentBuilder::new().build()`.
    pub fn new() -> Self {
        AgentBuilder::new().build()
    }

    /// Set a extra header field that will be present in all following requests using the agent.
    ///
    /// This is useful for cases like auth, where we do a number of requests before getting
    /// some credential that later must be presented in a header.
    ///
    /// Notice that fixed headers can also be set in the `AgentBuilder`.
    ///
    /// ```
    /// let mut agent = ureq::agent();
    ///
    /// agent.set("X-API-Key", "foobar");
    /// agent.set("Accept", "text/plain");
    ///
    /// let r = agent
    ///     .get("/my-page")
    ///     .call();
    /// ```
    pub fn set(&mut self, header: &str, value: &str) {
        header::add_header(&mut self.headers, Header::new(header, value));
    }

    /// Request by providing the HTTP verb such as `GET`, `POST`...
    ///
    /// ```
    /// let agent = ureq::agent();
    ///
    /// let r = agent
    ///     .request("GET", "/my_page")
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn request(&self, method: &str, path: &str) -> Request {
        Request::new(self.clone(), method.into(), path.into())
    }

    /// Store a cookie in this agent.
    ///
    /// ```
    /// let agent = ureq::agent();
    ///
    /// let cookie = ureq::Cookie::build("name", "value")
    ///   .secure(true)
    ///   .finish();
    /// agent.set_cookie(cookie, &"https://example.com/".parse().unwrap());
    /// ```
    #[cfg(feature = "cookies")]
    pub fn set_cookie(&self, cookie: Cookie<'static>, url: &Url) {
        self.state
            .cookie_tin
            .store_response_cookies(Some(cookie).into_iter(), url);
    }

    /// Make a GET request from this agent.
    pub fn get(&self, path: &str) -> Request {
        self.request("GET", path)
    }

    /// Make a HEAD request from this agent.
    pub fn head(&self, path: &str) -> Request {
        self.request("HEAD", path)
    }

    /// Make a POST request from this agent.
    pub fn post(&self, path: &str) -> Request {
        self.request("POST", path)
    }

    /// Make a PUT request from this agent.
    pub fn put(&self, path: &str) -> Request {
        self.request("PUT", path)
    }

    /// Make a DELETE request from this agent.
    pub fn delete(&self, path: &str) -> Request {
        self.request("DELETE", path)
    }

    /// Make a TRACE request from this agent.
    pub fn trace(&self, path: &str) -> Request {
        self.request("TRACE", path)
    }

    /// Make a OPTIONS request from this agent.
    pub fn options(&self, path: &str) -> Request {
        self.request("OPTIONS", path)
    }

    /// Make a PATCH request from this agent.
    pub fn patch(&self, path: &str) -> Request {
        self.request("PATCH", path)
    }
}

impl AgentBuilder {
    pub fn new() -> Self {
        AgentBuilder {
            headers: vec![],
            config: AgentConfig {
                max_idle_connections: crate::pool::DEFAULT_MAX_IDLE_CONNECTIONS,
                max_idle_connections_per_host: crate::pool::DEFAULT_MAX_IDLE_CONNECTIONS_PER_HOST,
                proxy: None,
                timeout_connect: Some(Duration::from_secs(30)),
                timeout_read: None,
                timeout_write: None,
                timeout: None,
                redirects: 5,
                #[cfg(feature = "tls")]
                tls_config: None,
            },
            resolver: StdResolver.into(),
            #[cfg(feature = "cookies")]
            cookie_store: None,
        }
    }

    /// Create a new agent.
    // Note: This could take &self as the first argument, allowing one
    // AgentBuilder to be used multiple times, except CookieStore does
    // not implement clone, so we have to give ownership to the newly
    // built Agent.
    pub fn build(self) -> Agent {
        let config = Arc::new(self.config);
        Agent {
            headers: self.headers,
            state: Arc::new(AgentState {
                pool: ConnectionPool::new_with_limits(
                    config.max_idle_connections,
                    config.max_idle_connections_per_host,
                ),
                proxy: config.proxy.clone(),
                #[cfg(feature = "cookies")]
                cookie_tin: CookieTin::new(
                    self.cookie_store.unwrap_or_else(|| CookieStore::default()),
                ),
                resolver: self.resolver,
            }),
            config,
        }
    }

    /// Set a header field that will be present in all requests using the agent.
    ///
    /// ```
    /// let agent = ureq::builder()
    ///     .set("X-API-Key", "foobar")
    ///     .set("Accept", "text/plain")
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my-page")
    ///     .call();
    ///
    ///  if let Ok(resp) = r {
    ///      println!("yay got {}", resp.into_string().unwrap());
    ///  } else {
    ///      println!("Oh no error!");
    ///  }
    /// ```
    pub fn set(mut self, header: &str, value: &str) -> Self {
        header::add_header(&mut self.headers, Header::new(header, value));
        self
    }

    /// Set the proxy server to use for all connections from this Agent.
    ///
    /// Example:
    /// ```
    /// let proxy = ureq::Proxy::new("user:password@cool.proxy:9090").unwrap();
    /// let agent = ureq::AgentBuilder::new()
    ///     .proxy(proxy)
    ///     .build();
    /// ```
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.config.proxy = Some(proxy);
        self
    }

    /// Sets the maximum number of connections allowed in the connection pool.
    /// By default, this is set to 100. Setting this to zero would disable
    /// connection pooling.
    ///
    /// ```
    /// let agent = ureq::AgentBuilder::new().max_idle_connections(200).build();
    /// ```
    pub fn max_idle_connections(mut self, max: usize) -> Self {
        self.config.max_idle_connections = max;
        self
    }

    /// Sets the maximum number of connections per host to keep in the
    /// connection pool. By default, this is set to 1. Setting this to zero
    /// would disable connection pooling.
    ///
    /// ```
    /// let agent = ureq::AgentBuilder::new().max_idle_connections_per_host(200).build();
    /// ```
    pub fn max_idle_connections_per_host(mut self, max: usize) -> Self {
        self.config.max_idle_connections_per_host = max;
        self
    }

    /// Configures a custom resolver to be used by this agent. By default,
    /// address-resolution is done by std::net::ToSocketAddrs. This allows you
    /// to override that resolution with your own alternative. Useful for
    /// testing and special-cases like DNS-based load balancing.
    ///
    /// A `Fn(&str) -> io::Result<Vec<SocketAddr>>` is a valid resolver,
    /// passing a closure is a simple way to override. Note that you might need
    /// explicit type `&str` on the closure argument for type inference to
    /// succeed.
    /// ```
    /// use std::net::ToSocketAddrs;
    ///
    /// let mut agent = ureq::AgentBuilder::new()
    ///    .resolver(|addr: &str| match addr {
    ///       "example.com" => Ok(vec![([127,0,0,1], 8096).into()]),
    ///       addr => addr.to_socket_addrs().map(Iterator::collect),
    ///    })
    ///    .build();
    /// ```
    pub fn resolver(mut self, resolver: impl crate::Resolver + 'static) -> Self {
        self.resolver = resolver.into();
        self
    }

    /// Timeout for the socket connection to be successful.
    /// If both this and `.timeout()` are both set, `.timeout_connect()`
    /// takes precedence.
    ///
    /// The default is 30 seconds.
    ///
    /// ```
    /// let agent = ureq::builder()
    ///     .timeout_connect(std::time::Duration::from_secs(1))
    ///     .build();
    /// let r = agent.get("/my_page").call();
    /// ```
    pub fn timeout_connect(mut self, timeout: Duration) -> Self {
        self.config.timeout_connect = Some(timeout);
        self
    }

    /// Timeout for the individual reads of the socket.
    /// If both this and `.timeout()` are both set, `.timeout()`
    /// takes precedence.
    ///
    /// The default is `0`, which means it can block forever.
    ///
    /// ```
    /// let agent = ureq::builder()
    ///     .timeout_read(std::time::Duration::from_secs(1))
    ///     .build();
    /// let r = agent.get("/my_page").call();
    /// ```
    pub fn timeout_read(mut self, timeout: Duration) -> Self {
        self.config.timeout_read = Some(timeout);
        self
    }

    /// Timeout for the individual writes to the socket.
    /// If both this and `.timeout()` are both set, `.timeout()`
    /// takes precedence.
    ///
    /// The default is `0`, which means it can block forever.
    ///
    /// ```
    /// let agent = ureq::builder()
    ///     .timeout_write(std::time::Duration::from_secs(1))
    ///     .build();
    /// let r = agent.get("/my_page").call();
    /// ```
    pub fn timeout_write(mut self, timeout: Duration) -> Self {
        self.config.timeout_write = Some(timeout);
        self
    }

    /// Timeout for the overall request, including DNS resolution, connection
    /// time, redirects, and reading the response body. Slow DNS resolution
    /// may cause a request to exceed the timeout, because the DNS request
    /// cannot be interrupted with the available APIs.
    ///
    /// This takes precedence over `.timeout_read()` and `.timeout_write()`, but
    /// not `.timeout_connect()`.
    ///
    /// ```
    /// // wait max 1 second for whole request to complete.
    /// let agent = ureq::builder()
    ///     .timeout(std::time::Duration::from_secs(1))
    ///     .build();
    /// let r = agent.get("/my_page").call();
    /// ```
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self
    }

    /// How many redirects to follow.
    ///
    /// Defaults to `5`. Set to `0` to avoid redirects and instead
    /// get a response object with the 3xx status code.
    ///
    /// If the redirect count hits this limit (and it's > 0), TooManyRedirects is returned.
    ///
    /// ```
    /// let r = ureq::builder()
    ///     .redirects(10)
    ///     .build()
    ///     .get("/my_page")
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn redirects(mut self, n: u32) -> Self {
        self.config.redirects = n;
        self
    }

    /// Set the TLS client config to use for the connection. See [`ClientConfig`](https://docs.rs/rustls/latest/rustls/struct.ClientConfig.html).
    ///
    /// Example:
    /// ```
    /// let tls_config = std::sync::Arc::new(rustls::ClientConfig::new());
    /// let agent = ureq::builder()
    ///     .set_tls_config(tls_config.clone())
    ///     .build();
    /// let req = agent.post("https://cool.server");
    /// ```
    #[cfg(feature = "tls")]
    pub fn set_tls_config(mut self, tls_config: Arc<rustls::ClientConfig>) -> Self {
        self.config.tls_config = Some(TLSClientConfig(tls_config));
        self
    }
}

#[cfg(feature = "tls")]
#[derive(Clone)]
pub(crate) struct TLSClientConfig(pub(crate) Arc<rustls::ClientConfig>);

#[cfg(feature = "tls")]
impl std::fmt::Debug for TLSClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TLSClientConfig").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    ///////////////////// AGENT TESTS //////////////////////////////

    #[test]
    fn agent_implements_send_and_sync() {
        let _agent: Box<dyn Send> = Box::new(AgentBuilder::new().build());
        let _agent: Box<dyn Sync> = Box::new(AgentBuilder::new().build());
    }
}
