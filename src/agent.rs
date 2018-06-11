use std::str::FromStr;
use std::sync::Mutex;

use header::{Header, add_header};
use util::*;

// to get to share private fields
include!("request.rs");
include!("response.rs");
include!("conn.rs");

#[derive(Debug, Default, Clone)]
pub struct Agent {
    pub headers: Vec<Header>,
    pub pool: Arc<Mutex<Option<ConnectionPool>>>,
}

impl Agent {
    pub fn new() -> Agent {
        Default::default()
    }

    /// Create a new agent after treating it as a builder.
    /// This actually clones the internal state to a new one and instantiates
    /// a new connection pool that is reused between connects.
    pub fn build(&self) -> Self {
        Agent {
            headers: self.headers.clone(),
            pool: Arc::new(Mutex::new(Some(ConnectionPool::new()))),
        }
    }

    /// Set a header field that will be present in all requests using the agent.
    ///
    /// ```
    /// let agent = ureq::agent()
    ///     .set("X-API-Key", "foobar")
    ///     .set("Accept", "application/json")
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my-page")
    ///     .call();
    ///
    ///  if r.ok() {
    ///      println!("yay got {}", r.into_json().unwrap());
    ///  } else {
    ///      println!("Oh no error!");
    ///  }
    /// ```
    pub fn set<K, V>(&mut self, header: K, value: V) -> &mut Agent
    where
        K: Into<String>,
        V: Into<String>,
    {
        add_agent_header(self, header.into(), value.into());
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
    ///     .set_map(map!{
    ///         "X-API-Key" => "foobar",
    ///         "Accept" => "application/json"
    ///     })
    ///     .build();
    ///
    /// let r = agent
    ///     .get("/my_page")
    ///     .call();
    ///
    /// if r.ok() {
    ///     println!("yay got {}", r.into_json().unwrap());
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
            add_agent_header(self, k.into(), v.into());
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
        add_header(header, &mut self.headers);
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

    pub fn get<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("GET", path)
    }
    pub fn head<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("HEAD", path)
    }
    pub fn post<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("POST", path)
    }
    pub fn put<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("PUT", path)
    }
    pub fn delete<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("DELETE", path)
    }
    pub fn trace<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("TRACE", path)
    }
    pub fn options<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("OPTIONS", path)
    }
    pub fn connect<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("CONNECT", path)
    }
    pub fn patch<S>(&self, path: S) -> Request
    where
        S: Into<String>,
    {
        self.request("PATCH", path)
    }
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

    //////////////////// RESPONSE TESTS /////////////////////////////

    #[test]
    fn content_type_without_charset() {
        let s = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\nOK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("application/json", resp.content_type());
    }

    #[test]
    fn content_type_with_charset() {
        let s = "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=iso-8859-4\r\n\r\nOK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("application/json", resp.content_type());
    }

    #[test]
    fn content_type_default() {
        let s = "HTTP/1.1 200 OK\r\n\r\nOK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("text/plain", resp.content_type());
    }

    #[test]
    fn charset() {
        let s = "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=iso-8859-4\r\n\r\nOK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("iso-8859-4", resp.charset());
    }

    #[test]
    fn charset_default() {
        let s = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\nOK";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("utf-8", resp.charset());
    }

    #[test]
    fn chunked_transfer() {
        let s = "HTTP/1.1 200 OK\r\nTransfer-Encoding: Chunked\r\n\r\n3\r\nhel\r\nb\r\nlo world!!!\r\n0\r\n\r\n";
        let resp = s.parse::<Response>().unwrap();
        assert_eq!("hello world!!!", resp.into_string().unwrap());
    }

    #[test]
    fn parse_simple_json() {
        let s = format!("HTTP/1.1 200 OK\r\n\r\n{{\"hello\":\"world\"}}");
        let resp = s.parse::<Response>().unwrap();
        let v = resp.into_json().unwrap();
        assert_eq!(
            v,
            "{\"hello\":\"world\"}"
                .parse::<serde_json::Value>()
                .unwrap()
        );
    }

    #[test]
    fn parse_borked_header() {
        let s = format!("HTTP/1.1 BORKED\r\n");
        let resp: Response = s.parse::<Response>().unwrap_err().into();
        assert_eq!(resp.http_version(), "HTTP/1.1");
        assert_eq!(*resp.status(), 500);
        assert_eq!(resp.status_text(), "Bad Status");
        assert_eq!(resp.content_type(), "text/plain");
        let v = resp.into_string().unwrap();
        assert_eq!(v, "Bad Status\n");
    }

}
