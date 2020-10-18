#[cfg(feature = "cookie")]
use cookie::Cookie;
#[cfg(feature = "cookie")]
use cookie_store::CookieStore;
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(feature = "cookie")]
use url::Url;

use crate::header::{self, Header};
use crate::pool::ConnectionPool;
use crate::proxy::Proxy;
use crate::request::Request;
use crate::resolve::ArcResolver;

#[derive(Debug, Default)]
pub struct AgentBuilder {
    headers: Vec<Header>,
    proxy: Option<Proxy>,
    max_idle_connections: usize,
    max_idle_connections_per_host: usize,
    /// Cookies saved between requests.
    /// Invariant: All cookies must have a nonempty domain and path.
    #[cfg(feature = "cookie")]
    jar: CookieStore,
    resolver: ArcResolver,
}

impl Default for Agent {
    fn default() -> Self {
        AgentBuilder::new().build()
    }
}

/// Agents keep state between requests.
///
/// By default, no state, such as cookies, is kept between requests.
/// But by creating an agent as entry point for the request, we
/// can keep a state.
///
/// ```
/// let agent = ureq::agent();
///
/// let auth = agent
///     .post("/login")
///     .auth("martin", "rubbermashgum")
///     .call(); // blocks. puts auth cookies in agent.
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
#[derive(Debug, Clone)]
pub struct Agent {
    /// Copied into each request of this agent.
    pub(crate) headers: Vec<Header>,
    /// Reused agent state for repeated requests from this agent.
    pub(crate) state: Arc<Mutex<AgentState>>,
}

/// Container of the state
///
/// *Internal API*.
#[derive(Debug, Default)]
pub(crate) struct AgentState {
    /// Reused connections between requests.
    pub(crate) pool: ConnectionPool,
    pub(crate) proxy: Option<Proxy>,
    /// Cookies saved between requests.
    /// Invariant: All cookies must have a nonempty domain and path.
    #[cfg(feature = "cookie")]
    pub(crate) jar: CookieStore,
    pub(crate) resolver: ArcResolver,
}

impl AgentState {
    pub(crate) fn pool(&mut self) -> &mut ConnectionPool {
        &mut self.pool
    }
}

impl Agent {
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
        Request::new(&self, method.into(), path.into())
    }

    /// Gets a cookie in this agent by name. Cookies are available
    /// either by setting it in the agent, or by making requests
    /// that `Set-Cookie` in the agent.
    ///
    /// Note that this will return any cookie for the given name,
    /// regardless of which host and path that cookie was set on.
    ///
    /// ```
    /// let agent = ureq::agent();
    ///
    /// agent.get("http://www.google.com").call();
    ///
    /// assert!(agent.cookie("NID").is_some());
    /// ```
    #[cfg(feature = "cookie")]
    pub fn cookie(&self, name: &str) -> Option<Cookie<'static>> {
        let state = self.state.lock().unwrap();
        let first_found = state.jar.iter_any().find(|c| c.name() == name);
        if let Some(first_found) = first_found {
            let c: &Cookie = &*first_found;
            Some(c.clone())
        } else {
            None
        }
    }

    /// Set a cookie in this agent.
    ///
    /// Cookies without a domain, or with a malformed domain or path,
    /// will be silently ignored.
    ///
    /// ```
    /// let agent = ureq::agent();
    ///
    /// let cookie = ureq::Cookie::build("name", "value")
    ///   .domain("example.com")
    ///   .path("/")
    ///   .secure(true)
    ///   .finish();
    /// agent.set_cookie(cookie);
    /// ```
    #[cfg(feature = "cookie")]
    pub fn set_cookie(&self, cookie: Cookie<'static>) {
        let mut cookie = cookie.clone();
        if cookie.domain().is_none() {
            return;
        }

        if cookie.path().is_none() {
            cookie.set_path("/");
        }
        let path = cookie.path().unwrap();
        let domain = cookie.domain().unwrap();

        let fake_url: Url = match format!("http://{}{}", domain, path).parse() {
            Ok(u) => u,
            Err(_) => return,
        };
        let mut state = self.state.lock().unwrap();
        let cs_cookie = match cookie_store::Cookie::try_from_raw_cookie(&cookie, &fake_url) {
            Ok(c) => c,
            Err(_) => return,
        };
        state.jar.insert(cs_cookie, &fake_url).ok();
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
    pub fn new() -> AgentBuilder {
        AgentBuilder {
            max_idle_connections: 100,
            max_idle_connections_per_host: 1,
            ..Default::default()
        }
    }

    /// Create a new agent.
    // Note: This could take &self as the first argument, allowing one
    // AgentBuilder to be used multiple times, except CookieStore does
    // not implement clone, so we have to give ownership to the newly
    // built Agent.
    pub fn build(self) -> Agent {
        Agent {
            headers: self.headers.clone(),
            state: Arc::new(Mutex::new(AgentState {
                pool: ConnectionPool::new(
                    self.max_idle_connections,
                    self.max_idle_connections_per_host,
                ),
                proxy: self.proxy.clone(),
                #[cfg(feature = "cookie")]
                jar: self.jar,
                resolver: self.resolver,
            })),
        }
    }

    /// Set a header field that will be present in all requests using the agent.
    ///
    /// ```
    /// let agent = ureq::AgentBuilder::new()
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

    /// Basic auth that will be present in all requests using the agent.
    ///
    /// ```
    /// let agent = ureq::AgentBuilder::new()
    ///     .auth("martin", "rubbermashgum")
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my_page")
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn auth(self, user: &str, pass: &str) -> Self {
        let pass = basic_auth(user, pass);
        self.auth_kind("Basic", &pass)
    }

    /// Auth of other kinds such as `Digest`, `Token` etc, that will be present
    /// in all requests using the agent.
    ///
    /// ```
    /// // sets a header "Authorization: token secret"
    /// let agent = ureq::AgentBuilder::new()
    ///     .auth_kind("token", "secret")
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my_page")
    ///     .call();
    /// ```
    pub fn auth_kind(self, kind: &str, pass: &str) -> Self {
        let value = format!("{} {}", kind, pass);
        self.set("Authorization", &value)
    }
    /// Sets the maximum number of connections allowed in the connection pool.
    /// By default, this is set to 100. Setting this to zero would disable
    /// connection pooling.
    ///
    /// ```
    /// let agent = ureq::AgentBuilder::new().max_pool_connections(200).build();
    /// ```
    pub fn max_pool_connections(mut self, max: usize) -> Self {
        self.max_idle_connections = max;
        self
    }

    /// Sets the maximum number of connections per host to keep in the
    /// connection pool. By default, this is set to 1. Setting this to zero
    /// would disable connection pooling.
    ///
    /// ```
    /// let agent = ureq::AgentBuilder::new().max_pool_connections_per_host(200).build();
    /// ```
    pub fn max_pool_connections_per_host(mut self, max: usize) -> Self {
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
        self.proxy = Some(proxy);
        self
    }
}

pub(crate) fn basic_auth(user: &str, pass: &str) -> String {
    let safe = match user.find(':') {
        Some(idx) => &user[..idx],
        None => user,
    };
    base64::encode(&format!("{}:{}", safe, pass))
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

    #[test]
    #[cfg(any(feature = "tls", feature = "native-tls"))]
    fn agent_pool() {
        use std::io::Read;

        let agent = crate::agent();
        let url = "https://ureq.s3.eu-central-1.amazonaws.com/sherlock.txt";
        // req 1
        let resp = agent.get(url).call().unwrap();
        let mut reader = resp.into_reader();
        let mut buf = vec![];
        // reading the entire content will return the connection to the pool
        reader.read_to_end(&mut buf).unwrap();

        fn poolsize(agent: &Agent) -> usize {
            let mut state = agent.state.lock().unwrap();
            state.pool().len()
        }
        assert_eq!(poolsize(&agent), 1);

        // req 2 should be done with a reused connection
        let resp = agent.get(url).call().unwrap();
        assert_eq!(poolsize(&agent), 0);
        let mut reader = resp.into_reader();
        let mut buf = vec![];
        reader.read_to_end(&mut buf).unwrap();
    }
}
