use cookie::{Cookie, CookieJar};
use error::Error;
use pool::ConnectionPool;
use response::{self, Response};
use std::sync::Mutex;

use header::{add_header, get_all_headers, get_header, has_header, Header};

// to get to share private fields
include!("request.rs");
include!("unit.rs");

/// Agents keep state between requests.
///
/// By default, no state, such as cookies, is kept between requests.
/// But by creating an agent as entry point for the request, we
/// can keep a state.
///
/// ```
/// let agent = ureq::agent().build();
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
    headers: Vec<Header>,
    state: Arc<Mutex<Option<AgentState>>>,
}

#[derive(Debug)]
pub struct AgentState {
    pool: ConnectionPool,
    jar: CookieJar,
}

impl AgentState {
    fn new() -> Self {
        AgentState {
            pool: ConnectionPool::new(),
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
    pub fn set<K, V>(&mut self, header: K, value: V) -> &mut Agent
    where
        K: Into<String>,
        V: Into<String>,
    {
        let s = format!("{}: {}", header.into(), value.into());
        let header = s.parse::<Header>().expect("Failed to parse header");
        add_header(&mut self.headers, header);
        self
    }

    /// Set many headers that will be present in all requests using the agent.
    ///
    /// ```
    /// #[macro_use]
    /// extern crate ureq;
    ///
    /// fn main() {
    /// let agent = ureq::agent()
    ///     .set_map(map! {
    ///         "X-API-Key" => "foobar",
    ///         "Accept" => "text/plain"
    ///     })
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my_page")
    ///     .call();
    ///
    /// if r.ok() {
    ///     println!("yay got {}", r.into_string().unwrap());
    /// }
    /// }
    /// ```
    pub fn set_map<K, V, I>(&mut self, headers: I) -> &mut Agent
    where
        K: Into<String>,
        V: Into<String>,
        I: IntoIterator<Item = (K, V)>,
    {
        for (k, v) in headers.into_iter() {
            let s = format!("{}: {}", k.into(), v.into());
            let header = s.parse::<Header>().expect("Failed to parse header");
            add_header(&mut self.headers, header);
        }
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
    pub fn auth<S, T>(&mut self, user: S, pass: T) -> &mut Agent
    where
        S: Into<String>,
        T: Into<String>,
    {
        let u = user.into();
        let p = pass.into();
        let pass = basic_auth(&u, &p);
        self.auth_kind("Basic", pass)
    }

    /// Auth of other kinds such as `Digest`, `Token` etc, that will be present
    /// in all requests using the agent.
    ///
    /// ```
    /// let agent = ureq::agent()
    ///     .auth_kind("token", "secret")
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my_page")
    ///     .call();
    /// ```
    pub fn auth_kind<S, T>(&mut self, kind: S, pass: T) -> &mut Agent
    where
        S: Into<String>,
        T: Into<String>,
    {
        let s = format!("Authorization: {} {}", kind.into(), pass.into());
        let header = s.parse::<Header>().expect("Failed to parse header");
        add_header(&mut self.headers, header);
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
    pub fn request<M, S>(&self, method: M, path: S) -> Request
    where
        M: Into<String>,
        S: Into<String>,
    {
        Request::new(&self, method.into(), path.into())
    }

    /// Gets a cookie in this agent by name. Cookies are available
    /// either by setting it in the agent, or by making requests
    /// that `Set-Cookie` in the agent.
    ///
    /// ```
    /// let agent = ureq::agent().build();
    ///
    /// agent.get("http://www.google.com").call();
    ///
    /// assert!(agent.cookie("NID").is_some());
    /// ```
    pub fn cookie(&self, name: &str) -> Option<Cookie<'static>> {
        let state = self.state.lock().unwrap();
        state
            .as_ref()
            .and_then(|state| state.jar.get(name))
            .map(|c| c.clone())
    }

    /// Set a cookie in this agent.
    ///
    /// ```
    /// let agent = ureq::agent().build();
    ///
    /// let cookie = ureq::Cookie::new("name", "value");
    /// agent.set_cookie(cookie);
    /// ```
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
    pub fn get<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("GET", path)
    }

    /// Make a HEAD request from this agent.
    pub fn head<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("HEAD", path)
    }

    /// Make a POST request from this agent.
    pub fn post<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("POST", path)
    }

    /// Make a PUT request from this agent.
    pub fn put<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("PUT", path)
    }

    /// Make a DELETE request from this agent.
    pub fn delete<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("DELETE", path)
    }

    /// Make a TRACE request from this agent.
    pub fn trace<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("TRACE", path)
    }

    /// Make a OPTIONS request from this agent.
    pub fn options<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("OPTIONS", path)
    }

    /// Make a CONNECT request from this agent.
    pub fn connect<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("CONNECT", path)
    }

    /// Make a PATCH request from this agent.
    pub fn patch<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("PATCH", path)
    }

    #[cfg(test)]
    pub fn state(&self) -> &Arc<Mutex<Option<AgentState>>> {
        &self.state
    }
}

fn basic_auth(user: &str, pass: &str) -> String {
    let safe = match user.find(":") {
        Some(idx) => &user[..idx],
        None => user,
    };
    ::base64::encode(&format!("{}:{}", safe, pass))
}

#[cfg(test)]
mod tests {
    use super::*;

    ///////////////////// AGENT TESTS //////////////////////////////

    #[test]
    fn agent_implements_send() {
        let mut agent = Agent::new();
        ::std::thread::spawn(move || {
            agent.set("Foo", "Bar");
        });
    }

    //////////////////// REQUEST TESTS /////////////////////////////

    #[test]
    fn request_implements_send() {
        let agent = Agent::new();
        let mut request = Request::new(&agent, "GET".to_string(), "/foo".to_string());
        ::std::thread::spawn(move || {
            request.set("Foo", "Bar");
        });
    }

}
