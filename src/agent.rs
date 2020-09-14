#[cfg(feature = "cookie")]
use cookie::{Cookie, CookieJar};
use std::sync::Arc;
use std::sync::Mutex;

use crate::header::{self, Header};
use crate::pool::ConnectionPool;
use crate::request::Request;

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
    pub(crate) state: Arc<Mutex<Option<AgentState>>>,
}

/// Container of the state
///
/// *Internal API*.
#[derive(Debug)]
pub(crate) struct AgentState {
    /// Reused connections between requests.
    pub(crate) pool: ConnectionPool,
    /// Cookies saved between requests.
    #[cfg(feature = "cookie")]
    pub(crate) jar: CookieJar,
}

impl AgentState {
    fn new() -> Self {
        AgentState {
            pool: ConnectionPool::new(),
            #[cfg(feature = "cookie")]
            jar: CookieJar::new(),
        }
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
            state: Arc::new(Mutex::new(Some(AgentState::new()))),
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
        let mut optional_state = self.state.lock().unwrap();
        if let Some(state) = optional_state.as_mut() {
            state.pool.set_max_idle_connections(max_connections);
        }
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
        let mut optional_state = self.state.lock().unwrap();
        if let Some(state) = optional_state.as_mut() {
            state
                .pool
                .set_max_idle_connections_per_host(max_connections);
        }
    }

    /// Gets a cookie in this agent by name. Cookies are available
    /// either by setting it in the agent, or by making requests
    /// that `Set-Cookie` in the agent.
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
        state
            .as_ref()
            .and_then(|state| state.jar.get(name))
            .cloned()
    }

    /// Set a cookie in this agent.
    ///
    /// ```
    /// let agent = ureq::agent();
    ///
    /// let cookie = ureq::Cookie::new("name", "value");
    /// agent.set_cookie(cookie);
    /// ```
    #[cfg(feature = "cookie")]
    pub fn set_cookie(&self, cookie: Cookie<'static>) {
        let mut state = self.state.lock().unwrap();
        match state.as_mut() {
            None => (),
            Some(state) => {
                state.jar.add_original(cookie);
            }
        }
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

    /// Make a CONNECT request from this agent.
    pub fn connect(&self, path: &str) -> Request {
        self.request("CONNECT", path)
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
            let mut lock = agent.state.lock().unwrap();
            let state = lock.as_mut().unwrap();
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
