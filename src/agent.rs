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
/// if !auth.ok() {
///     println!("Noes!");
/// }
///
/// let secret = agent
///     .get("/my-protected-page")
///     .call(); // blocks and waits for request.
///
/// if !secret.ok() {
///     println!("Wot?!");
/// }
///
/// println!("Secret is: {}", secret.into_string().unwrap());
/// ```
#[derive(Debug, Default, Clone)]
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
    fn new() -> Self {
        Self::default()
    }
    pub fn pool(&mut self) -> &mut ConnectionPool {
        &mut self.pool
    }
}

impl Agent {
    /// Creates a new agent. Typically you'd use [`ureq::agent()`](fn.agent.html) to
    /// do this.
    ///
    /// ```
    /// let agent = ureq::Agent::new()
    ///     .set("X-My-Header", "Foo") // present on all requests from this agent
    ///     .build();
    ///
    /// agent.get("/foo");
    /// ```
    pub fn new() -> Agent {
        Default::default()
    }

    /// Create a new agent after treating it as a builder.
    /// This actually clones the internal state to a new one and instantiates
    /// a new connection pool that is reused between connects.
    pub fn build(&self) -> Self {
        Agent {
            headers: self.headers.clone(),
            state: Arc::new(Mutex::new(AgentState::new())),
        }
    }

    /// Set a header field that will be present in all requests using the agent.
    ///
    /// ```
    /// let agent = ureq::agent()
    ///     .set("X-API-Key", "foobar")
    ///     .set("Accept", "text/plain")
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my-page")
    ///     .call();
    ///
    ///  if r.ok() {
    ///      println!("yay got {}", r.into_string().unwrap());
    ///  } else {
    ///      println!("Oh no error!");
    ///  }
    /// ```
    pub fn set(&mut self, header: &str, value: &str) -> &mut Agent {
        header::add_header(&mut self.headers, Header::new(header, value));
        self
    }

    /// Basic auth that will be present in all requests using the agent.
    ///
    /// ```
    /// let agent = ureq::agent()
    ///     .auth("martin", "rubbermashgum")
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my_page")
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn auth(&mut self, user: &str, pass: &str) -> &mut Agent {
        let pass = basic_auth(user, pass);
        self.auth_kind("Basic", &pass)
    }

    /// Auth of other kinds such as `Digest`, `Token` etc, that will be present
    /// in all requests using the agent.
    ///
    /// ```
    /// // sets a header "Authorization: token secret"
    /// let agent = ureq::agent()
    ///     .auth_kind("token", "secret")
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my_page")
    ///     .call();
    /// ```
    pub fn auth_kind(&mut self, kind: &str, pass: &str) -> &mut Agent {
        let value = format!("{} {}", kind, pass);
        self.set("Authorization", &value);
        self
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
        Request::new(&self, method.into(), path.into())
    }

    /// Sets the maximum number of connections allowed in the connection pool.
    /// By default, this is set to 100. Setting this to zero would disable
    /// connection pooling.
    ///
    /// ```
    /// let agent = ureq::agent();
    /// agent.set_max_pool_connections(200);
    /// ```
    pub fn set_max_pool_connections(&self, max_connections: usize) {
        let mut state = self.state.lock().unwrap();
        state.pool.set_max_idle_connections(max_connections);
    }

    /// Sets the maximum number of connections per host to keep in the
    /// connection pool. By default, this is set to 1. Setting this to zero
    /// would disable connection pooling.
    ///
    /// ```
    /// let agent = ureq::agent();
    /// agent.set_max_pool_connections_per_host(10);
    /// ```
    pub fn set_max_pool_connections_per_host(&self, max_connections: usize) {
        let mut state = self.state.lock().unwrap();
        state
            .pool
            .set_max_idle_connections_per_host(max_connections);
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
    /// let mut agent = ureq::agent();
    /// agent.set_resolver(|addr: &str| match addr {
    ///    "example.com" => Ok(vec![([127,0,0,1], 8096).into()]),
    ///    addr => addr.to_socket_addrs().map(Iterator::collect),
    /// });
    /// ```
    pub fn set_resolver(&mut self, resolver: impl crate::Resolver + 'static) -> &mut Self {
        self.state.lock().unwrap().resolver = resolver.into();
        self
    }

    /// Set the proxy server to use for all connections from this Agent.
    ///
    /// Example:
    /// ```
    /// let proxy = ureq::Proxy::new("user:password@cool.proxy:9090").unwrap();
    /// let agent = ureq::agent()
    ///     .set_proxy(proxy)
    ///     .build();
    /// ```
    pub fn set_proxy(&mut self, proxy: Proxy) -> &mut Agent {
        let mut state = self.state.lock().unwrap();
        state.proxy = Some(proxy);
        drop(state);
        self
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
    use std::thread;

    ///////////////////// AGENT TESTS //////////////////////////////

    #[test]
    fn agent_implements_send() {
        let mut agent = Agent::new();
        thread::spawn(move || {
            agent.set("Foo", "Bar");
        });
    }

    #[test]
    #[cfg(any(feature = "tls", feature = "native-tls"))]
    fn agent_pool() {
        use std::io::Read;

        let agent = crate::agent();
        let url = "https://ureq.s3.eu-central-1.amazonaws.com/sherlock.txt";
        // req 1
        let resp = agent.get(url).call();
        assert!(resp.ok());
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
        let resp = agent.get(url).call();
        assert!(resp.ok());
        assert_eq!(poolsize(&agent), 0);
        let mut reader = resp.into_reader();
        let mut buf = vec![];
        reader.read_to_end(&mut buf).unwrap();
    }

    //////////////////// REQUEST TESTS /////////////////////////////

    #[test]
    fn request_implements_send() {
        let agent = Agent::new();
        let mut request = Request::new(&agent, "GET".to_string(), "/foo".to_string());
        thread::spawn(move || {
            request.set("Foo", "Bar");
        });
    }
}
