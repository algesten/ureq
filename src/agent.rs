use std::fmt;
use std::sync::Arc;

use url::Url;

use crate::pool::ConnectionPool;
use crate::proxy::Proxy;
use crate::request::Request;
use crate::resolve::{ArcResolver, StdResolver};
#[cfg(any(feature = "tls", feature = "native-tls"))]
use crate::stream::HttpsConnector;
use std::time::Duration;

#[cfg(feature = "cookies")]
use {
    crate::cookies::{CookieStoreGuard, CookieTin},
    cookie_store::CookieStore,
};

/// Accumulates options towards building an [Agent].
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
#[derive(Clone)]
pub(crate) struct AgentConfig {
    pub proxy: Option<Proxy>,
    pub timeout_connect: Option<Duration>,
    pub timeout_read: Option<Duration>,
    pub timeout_write: Option<Duration>,
    pub timeout: Option<Duration>,
    pub redirects: u32,
    pub user_agent: String,
    #[cfg(any(feature = "tls", feature = "native-tls"))]
    pub tls_config: Option<Arc<dyn HttpsConnector>>,
}

impl fmt::Debug for AgentConfig {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
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

    /// Make a request with the HTTP verb as a parameter.
    ///
    /// This allows making requests with verbs that don't have a dedicated
    /// method.
    ///
    /// If you've got an already-parsed [Url], try [request_url][Agent::request_url].
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// use ureq::Response;
    /// let agent = ureq::agent();
    ///
    /// let resp: Response = agent
    ///     .request("OPTIONS", "http://example.com/")
    ///     .call()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn request(&self, method: &str, path: &str) -> Request {
        Request::new(self.clone(), method.into(), path.into())
    }

    /// Make a request using an already-parsed [Url].
    ///
    /// This is useful if you've got a parsed Url from some other source, or if
    /// you want to parse the URL and then modify it before making the request.
    /// If you'd just like to pass a String or a `&str`, try [request][Agent::request].
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// use {url::Url, ureq::Response};
    /// let agent = ureq::agent();
    ///
    /// let mut url: Url = "http://example.com/some-page".parse().unwrap();
    /// url.set_path("/robots.txt");
    /// let resp: Response = agent
    ///     .request_url("GET", &url)
    ///     .call()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn request_url(&self, method: &str, url: &Url) -> Request {
        Request::new(self.clone(), method.into(), url.to_string())
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
    /// let mut file = File::create("cookies.json")?;
    /// agent.cookie_store().save_json(&mut file).unwrap();
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
                user_agent: format!("ureq/{}", env!("CARGO_PKG_VERSION")),
                #[cfg(any(feature = "tls", feature = "native-tls"))]
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
                cookie_tin: CookieTin::new(self.cookie_store.unwrap_or_else(CookieStore::default)),
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
    /// The default is no timeout. In other words, requests may block forever on reads by default.
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
    /// The default is no timeout. In other words, requests may block forever on writes by default.
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
    /// WARNING: for 307 and 308 redirects, this value is ignored for methods that have a body.
    /// You must handle 307 redirects yourself when sending a PUT, POST, PATCH, or DELETE request.
    ///
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let result = ureq::builder()
    ///     .redirects(1)
    ///     .build()
    ///     # ;
    /// # let result = ureq::agent()
    ///     .get("http://httpbin.org/status/301")
    ///     .call()?;
    /// assert_ne!(result.status(), 301);
    ///
    /// let result = ureq::post("http://httpbin.org/status/307")
    ///     .send_bytes(b"some data")?;
    /// assert_eq!(result.status(), 307);
    /// # Ok(())
    /// # }
    /// ```
    pub fn redirects(mut self, n: u32) -> Self {
        self.config.redirects = n;
        self
    }

    /// The user-agent header to associate with all requests from this agent by default.
    ///
    /// Defaults to `ureq/[VERSION]`. You can override the user-agent on an individual request by
    /// setting the `User-Agent` header when constructing the request.
    ///
    /// ```
    /// # #[cfg(feature = "json")]
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// let agent = ureq::builder()
    ///     .user_agent("ferris/1.0")
    ///     .build();
    ///
    /// // Uses agent's header
    /// let result: serde_json::Value =
    ///     agent.get("http://httpbin.org/headers").call()?.into_json()?;
    /// assert_eq!(&result["headers"]["User-Agent"], "ferris/1.0");
    ///
    /// // Overrides user-agent set on the agent
    /// let result: serde_json::Value = agent.get("http://httpbin.org/headers")
    ///     .set("User-Agent", "super-ferris/2.0")
    ///     .call()?.into_json()?;
    /// assert_eq!(&result["headers"]["User-Agent"], "super-ferris/2.0");
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "json"))]
    /// # fn main() {}
    /// ```
    pub fn user_agent(mut self, user_agent: &str) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }

    /// Configure TLS options to use when making HTTPS connections from this Agent.
    /// The parameter can be a
    /// [`rustls::ClientConfig`](https://docs.rs/rustls/0.19.1/rustls/struct.ClientConfig.html),
    /// a [`native_tls::TlsConnector`](https://docs.rs/native-tls/0.2.7/native_tls/struct.TlsConnector.html),
    /// or any type for which you implement the [HttpsConnector] trait.
    ///
    /// Example using rustls:
    /// ```
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// # #[cfg(feature = "tls")]
    /// # {
    /// use std::sync::Arc;
    /// let tls_config = Arc::new(rustls::ClientConfig::new());
    /// let agent = ureq::builder()
    ///     .tls_config(tls_config.clone())
    ///     .build();
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Example using native-tls:
    ///
    /// ```
    /// # #[cfg(feature = "native-tls")]
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// use std::sync::Arc;
    /// # #[cfg(feature = "native-tls")]
    /// let tls_connector = Arc::new(native_tls::TlsConnector::new().unwrap());
    /// # #[cfg(feature = "native-tls")]
    /// let agent = ureq::builder()
    ///     .tls_config(tls_connector.clone())
    ///     .build();
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(any(feature = "tls", feature = "native-tls"))]
    pub fn tls_config<T: HttpsConnector + 'static>(mut self, tls_config: T) -> Self {
        self.config.tls_config = Some(Arc::new(tls_config));
        self
    }

    /// Provide the cookie store to be used for all requests using this agent.
    ///
    /// This is useful in two cases. First when there is a need to persist cookies
    /// to some backing store, and second when there's a need to prepare the agent
    /// with some pre-existing cookies.
    ///
    /// Example
    /// ```no_run
    /// # fn main() -> Result<(), ureq::Error> {
    /// # ureq::is_test(true);
    /// use cookie_store::CookieStore;
    /// use std::fs::File;
    /// use std::io::BufReader;
    /// let file = File::open("cookies.json")?;
    /// let read = BufReader::new(file);
    ///
    /// // Read persisted cookies from cookies.json
    /// let my_store = CookieStore::load_json(read).unwrap();
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
