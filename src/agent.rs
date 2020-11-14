use std::sync::Arc;

use crate::pool::ConnectionPool;
use crate::proxy::Proxy;
use crate::request::Request;
use crate::resolve::{ArcResolver, StdResolver};
use std::time::Duration;

#[cfg(feature = "cookies")]
use {
    crate::cookies::{CookieStoreGuard, CookieTin},
    cookie_store::CookieStore,
};

#[derive(Debug)]
pub struct AgentBuilder {
    config: AgentConfig,
    max_idle_connections: usize,
    max_idle_connections_per_host: usize,
    /// Cookies saved between requests.
    /// Invariant: All cookies must have a nonempty domain and path.
    #[cfg(feature = "cookies")]
    cookie_store: Option<CookieStore>,
    resolver: ArcResolver,
}

/// Config as built by AgentBuilder and then static for the lifetime of the Agent.
#[derive(Debug, Clone)]
pub(crate) struct AgentConfig {
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
/// # fn main() -> Result<(), ureq::Error> {
/// # ureq::is_test(true);
/// let mut agent = ureq::agent();
///
/// agent
///     .post("http://example.com/login")
///     .call()?;
///
/// let secret = agent
///     .get("http://example.com/my-protected-page")
///     .call()?
///     .into_string()?;
///
///   println!("Secret is: {}", secret);
/// # Ok(())
/// # }
/// ```
///
/// Agent uses an inner Arc, so cloning an Agent results in an instance
/// that shares the same underlying connection pool and other state.
#[derive(Debug, Clone)]
pub struct Agent {
    pub(crate) config: Arc<AgentConfig>,
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

    /// Request by providing the HTTP verb such as `GET`, `POST`...
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let agent = ureq::agent();
    ///
    /// let resp = agent
    ///     .request("GET", "http://httpbin.org/status/200")
    ///     .call()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn request(&self, method: &str, path: &str) -> Request {
        Request::new(self.clone(), method.into(), path.into())
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

    /// Read access to the cookie store.
    ///
    /// Used to persist the cookies to an external writer.
    ///
    /// ```no_run
    /// use std::io::Write;
    /// use std::fs::File;
    ///
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let agent = ureq::agent();
    ///
    /// // Cookies set by www.google.com are stored in agent.
    /// agent.get("https://www.google.com/").call()?;
    ///
    /// // Saves (persistent) cookies
    /// let file = File::create("cookies.json")?;
    /// agent.cookie_store().save_json(&mut file)?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "cookies")]
    pub fn cookie_store(&self) -> CookieStoreGuard<'_> {
        self.state.cookie_tin.read_lock()
    }
}

const DEFAULT_MAX_IDLE_CONNECTIONS: usize = 100;
const DEFAULT_MAX_IDLE_CONNECTIONS_PER_HOST: usize = 1;

impl AgentBuilder {
    pub fn new() -> Self {
        AgentBuilder {
            config: AgentConfig {
                proxy: None,
                timeout_connect: Some(Duration::from_secs(30)),
                timeout_read: None,
                timeout_write: None,
                timeout: None,
                redirects: 5,
                #[cfg(feature = "tls")]
                tls_config: None,
            },
            max_idle_connections: DEFAULT_MAX_IDLE_CONNECTIONS,
            max_idle_connections_per_host: DEFAULT_MAX_IDLE_CONNECTIONS_PER_HOST,
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
        Agent {
            config: Arc::new(self.config),
            state: Arc::new(AgentState {
                pool: ConnectionPool::new_with_limits(
                    self.max_idle_connections,
                    self.max_idle_connections_per_host,
                ),
                #[cfg(feature = "cookies")]
                cookie_tin: CookieTin::new(
                    self.cookie_store.unwrap_or_else(|| CookieStore::default()),
                ),
                resolver: self.resolver,
            }),
        }
    }

    /// Set the proxy server to use for all connections from this Agent.
    ///
    /// Example:
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let proxy = ureq::Proxy::new("user:password@cool.proxy:9090")?;
    /// let agent = ureq::AgentBuilder::new()
    ///     .proxy(proxy)
    ///     .build();
    /// # Ok(())
    /// # }
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
    /// let agent = ureq::AgentBuilder::new()
    ///     .max_idle_connections(200)
    ///     .build();
    /// ```
    pub fn max_idle_connections(mut self, max: usize) -> Self {
        self.max_idle_connections = max;
        self
    }

    /// Sets the maximum number of connections per host to keep in the
    /// connection pool. By default, this is set to 1. Setting this to zero
    /// would disable connection pooling.
    ///
    /// ```
    /// let agent = ureq::AgentBuilder::new()
    ///     .max_idle_connections_per_host(200)
    ///     .build();
    /// ```
    pub fn max_idle_connections_per_host(mut self, max: usize) -> Self {
        self.max_idle_connections_per_host = max;
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
    /// use std::time::Duration;
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let agent = ureq::builder()
    ///     .timeout_connect(Duration::from_secs(1))
    ///     .build();
    /// let result = agent.get("http://httpbin.org/delay/20").call();
    /// # Ok(())
    /// # }
    /// ```
    pub fn timeout_connect(mut self, timeout: Duration) -> Self {
        self.config.timeout_connect = Some(timeout);
        self
    }

    /// Timeout for the individual reads of the socket.
    /// If both this and `.timeout()` are both set, `.timeout()`
    /// takes precedence.
    ///
    /// ```
    /// use std::time::Duration;
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let agent = ureq::builder()
    ///     .timeout_read(Duration::from_secs(1))
    ///     .build();
    /// let result = agent.get("http://httpbin.org/delay/20").call();
    /// # Ok(())
    /// # }
    /// ```
    pub fn timeout_read(mut self, timeout: Duration) -> Self {
        self.config.timeout_read = Some(timeout);
        self
    }

    /// Timeout for the individual writes to the socket.
    /// If both this and `.timeout()` are both set, `.timeout()`
    /// takes precedence.
    ///
    /// ```
    /// use std::time::Duration;
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let agent = ureq::builder()
    ///     .timeout_read(Duration::from_secs(1))
    ///     .build();
    /// let result = agent.get("http://httpbin.org/delay/20").call();
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// // wait max 1 second for whole request to complete.
    /// let agent = ureq::builder()
    ///     .timeout(std::time::Duration::from_secs(1))
    ///     .build();
    /// let result = agent.get("http://httpbin.org/delay/20").call();
    /// # Ok(())
    /// # }
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
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let result = ureq::builder()
    ///     .redirects(1)
    ///     .build()
    ///     .get("http://httpbin.org/redirect/3")
    ///     .call();
    /// # Ok(())
    /// # }
    /// ```
    pub fn redirects(mut self, n: u32) -> Self {
        self.config.redirects = n;
        self
    }

    /// Set the TLS client config to use for the connection. See [`ClientConfig`](https://docs.rs/rustls/latest/rustls/struct.ClientConfig.html).
    ///
    /// Example:
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// use std::sync::Arc;
    /// let tls_config = Arc::new(rustls::ClientConfig::new());
    /// let agent = ureq::builder()
    ///     .tls_config(tls_config.clone())
    ///     .build();
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "tls")]
    pub fn tls_config(mut self, tls_config: Arc<rustls::ClientConfig>) -> Self {
        self.config.tls_config = Some(TLSClientConfig(tls_config));
        self
    }

    /// Provide the cookie store to be used for all requests using this agent.
    ///
    /// This is useful in two cases. First when there is a need to persist cookies
    /// to some backing store, and second when there's a need to prepare the agent
    /// with some pre-existing cookies.
    ///
    /// Example
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// use cookie_store::CookieStore;
    /// use std::fs::File;
    /// use std::io::BufReader;
    /// let file = File::open("cookies.json")?;
    /// let read = BufReader::new(file);
    ///
    /// // Read persisted cookies from cookies.json
    /// let my_store = CookieStore::load_json(read)?;
    ///
    /// // Cookies will be used for requests done through agent.
    /// let agent = ureq::builder()
    ///     .cookie_store(my_store)
    ///     .build();
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "cookies")]
    pub fn cookie_store(mut self, cookie_store: CookieStore) -> Self {
        self.cookie_store = Some(cookie_store);
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
