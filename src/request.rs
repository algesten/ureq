use super::SerdeValue;
use qstring::QString;
use serde_json;
use std::io::empty;
use std::io::Cursor;
use std::sync::Arc;

lazy_static! {
    static ref URL_BASE: Url = { Url::parse("http://localhost/").expect("Failed to parse URL_BASE") };
}

/// Request instances are builders that creates a request.
///
/// ```
/// let mut request = ureq::get("https://www.google.com/");
///
/// let response = request
///     .query("foo", "bar baz") // add ?foo=bar%20baz
///     .call();                 // run the request
/// ```
#[derive(Clone, Default)]
pub struct Request {
    state: Arc<Mutex<Option<AgentState>>>,

    // via agent
    method: String,
    path: String,

    // from request itself
    headers: Vec<Header>,
    query: QString,
    timeout: u32,
    timeout_read: u32,
    timeout_write: u32,
    redirects: u32,
}

impl ::std::fmt::Debug for Request {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(
            f,
            "Request[{} {}, {}, {:?}]",
            self.method, self.path, self.query, self.headers
        )
    }
}

enum Payload {
    Empty,
    Text(String),
    JSON(SerdeValue),
    Reader(Box<Read + 'static>),
}

impl Default for Payload {
    fn default() -> Payload {
        Payload::Empty
    }
}

struct SizedReader {
    size: Option<usize>,
    reader: Box<Read + 'static>,
}

impl SizedReader {
    fn new(size: Option<usize>, reader: Box<Read + 'static>) -> Self {
        SizedReader { size, reader }
    }
}

impl Payload {
    fn into_read(self) -> SizedReader {
        match self {
            Payload::Empty => SizedReader::new(None, Box::new(empty())),
            Payload::Text(s) => {
                let bytes = s.into_bytes();
                let len = bytes.len();
                let cursor = Cursor::new(bytes);
                SizedReader::new(Some(len), Box::new(cursor))
            }
            Payload::JSON(v) => {
                let bytes = serde_json::to_vec(&v).expect("Bad JSON in payload");
                let len = bytes.len();
                let cursor = Cursor::new(bytes);
                SizedReader::new(Some(len), Box::new(cursor))
            }
            Payload::Reader(read) => SizedReader::new(None, read),
        }
    }
}

impl Request {
    fn new(agent: &Agent, method: String, path: String) -> Request {
        Request {
            state: Arc::clone(&agent.state),
            method,
            path,
            headers: agent.headers.clone(),
            redirects: 5,
            ..Default::default()
        }
    }

    /// "Builds" this request which is effectively the same as cloning.
    /// This is needed when we use a chain of request builders, but
    /// don't want to send the request at the end of the chain.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .set("X-Foo-Bar", "Baz")
    ///     .build();
    /// ```
    pub fn build(&self) -> Request {
        self.clone()
    }

    /// Executes the request and blocks the caller until done.
    ///
    /// Use `.timeout()` and `.timeout_read()` to avoid blocking forever.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .timeout(10_000) // max 10 seconds
    ///     .call();
    ///
    /// println!("{:?}", r);
    /// ```
    pub fn call(&mut self) -> Response {
        self.do_call(Payload::Empty)
    }

    fn do_call(&mut self, payload: Payload) -> Response {
        let mut state = self.state.lock().unwrap();
        self.to_url()
            .and_then(|url| {
                match state.as_mut() {
                    None =>
                        // create a one off pool/jar.
                        ConnectionPool::new().connect(
                            self,
                            &self.method,
                            &url,
                            self.redirects,
                            None,
                            payload.into_read(),
                        ),
                    Some(state) => {
                        let jar = &mut state.jar;
                        state.pool.connect(
                            self,
                            &self.method,
                            &url,
                            self.redirects,
                            Some(jar),
                            payload.into_read(),
                        )
                    },
                }
            })
            .unwrap_or_else(|e| e.into())
    }

    /// Send data a json value.
    ///
    /// The `Content-Length` header is implicitly set to the length of the serialized value.
    ///
    /// ```
    /// #[macro_use]
    /// extern crate ureq;
    ///
    /// fn main() {
    /// let r = ureq::post("/my_page")
    ///     .send_json(json!({ "name": "martin", "rust": true }));
    /// println!("{:?}", r);
    /// }
    /// ```
    pub fn send_json(&mut self, data: SerdeValue) -> Response {
        self.do_call(Payload::JSON(data))
    }

    /// Send data as a string.
    ///
    /// The `Content-Length` header is implicitly set to the length of the serialized value.
    ///
    /// ```
    /// let r = ureq::post("/my_page")
    ///     .content_type("text/plain")
    ///     .send_str("Hello World!");
    /// println!("{:?}", r);
    /// ```
    pub fn send_str<S>(&mut self, data: S) -> Response
    where
        S: Into<String>,
    {
        let text = data.into();
        self.do_call(Payload::Text(text))
    }

    /// Send data from a reader.
    ///
    /// The `Content-Length` header is not set because we can't know the length of the reader.
    ///
    ///
    pub fn send<R>(&mut self, reader: R) -> Response
    where
        R: Read + Send + 'static,
    {
        self.do_call(Payload::Reader(Box::new(reader)))
    }

    /// Set a header field.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .set("Accept", "application/json")
    ///     .call();
    ///
    ///  if r.ok() {
    ///      println!("yay got {}", r.into_json().unwrap());
    ///  } else {
    ///      println!("Oh no error!");
    ///  }
    /// ```
    pub fn set<K, V>(&mut self, header: K, value: V) -> &mut Request
    where
        K: Into<String>,
        V: Into<String>,
    {
        let s = format!("{}: {}", header.into(), value.into());
        let header = s.parse::<Header>().expect("Failed to parse header");
        add_header(header, &mut self.headers);
        self
    }

    /// Returns the value for a set header.
    ///
    /// ```
    /// let req = ureq::get("/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .build();
    /// assert_eq!("foobar", req.header("x-api-Key").unwrap());
    /// ```
    pub fn header<'a>(&self, name: &'a str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.is_name(name))
            .map(|h| h.value())
    }

    /// Tells if the header has been set.
    ///
    /// ```
    /// let req = ureq::get("/my_page")
    ///     .set("X-API-Key", "foobar")
    ///     .build();
    /// assert_eq!(true, req.has("x-api-Key"));
    /// ```
    pub fn has<'a>(&self, name: &'a str) -> bool {
        self.header(name).is_some()
    }

    /// All headers corresponding values for the give name, or empty vector.
    ///
    /// ```
    /// let req = ureq::get("/my_page")
    ///     .set("X-Forwarded-For", "1.2.3.4")
    ///     .set("X-Forwarded-For", "2.3.4.5")
    ///     .build();
    /// assert_eq!(req.all("x-forwarded-for"), vec![
    ///     "1.2.3.4",
    ///     "2.3.4.5",
    /// ]);
    /// ```
    pub fn all<'a>(&self, name: &'a str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|h| h.is_name(name))
            .map(|h| h.value())
            .collect()
    }

    /// Set many headers.
    ///
    /// ```
    /// #[macro_use]
    /// extern crate ureq;
    ///
    /// fn main() {
    /// let r = ureq::get("/my_page")
    ///     .set_map(map! {
    ///         "X-API-Key" => "foobar",
    ///         "Accept" => "application/json"
    ///     })
    ///     .call();
    ///
    /// if r.ok() {
    ///     println!("yay got {}", r.into_json().unwrap());
    /// }
    /// }
    /// ```
    pub fn set_map<K, V, I>(&mut self, headers: I) -> &mut Request
    where
        K: Into<String>,
        V: Into<String>,
        I: IntoIterator<Item = (K, V)>,
    {
        for (k, v) in headers.into_iter() {
            let s = format!("{}: {}", k.into(), v.into());
            let header = s.parse::<Header>().expect("Failed to parse header");
            add_header(header, &mut self.headers);
        }
        self
    }

    /// Set a query parameter.
    ///
    /// For example, to set `?format=json&dest=/login`
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .query("format", "json")
    ///     .query("dest", "/login")
    ///     .call();
    ///
    /// println!("{:?}", r);
    /// ```
    pub fn query<K, V>(&mut self, param: K, value: V) -> &mut Request
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.query.add_pair((param.into(), value.into()));
        self
    }

    /// Set many query parameters.
    ///
    /// For example, to set `?format=json&dest=/login`
    ///
    /// ```
    /// #[macro_use]
    /// extern crate ureq;
    ///
    /// fn main() {
    /// let r = ureq::get("/my_page")
    ///     .query_map(map! {
    ///         "format" => "json",
    ///         "dest" => "/login"
    ///     })
    ///     .call();
    ///
    /// println!("{:?}", r);
    /// }
    /// ```
    pub fn query_map<K, V, I>(&mut self, params: I) -> &mut Request
    where
        K: Into<String>,
        V: Into<String>,
        I: IntoIterator<Item = (K, V)>,
    {
        for (k, v) in params.into_iter() {
            self.query.add_pair((k.into(), v.into()));
        }
        self
    }

    /// Set query parameters as a string.
    ///
    /// For example, to set `?format=json&dest=/login`
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .query_str("?format=json&dest=/login")
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn query_str<S>(&mut self, query: S) -> &mut Request
    where
        S: Into<String>,
    {
        let s = query.into();
        self.query.add_str(&s);
        self
    }

    /// Set the `Content-Type` header.
    ///
    /// The default is `application/json`.
    ///
    /// As a short-hand the `.content_type()` method accepts the
    /// canonicalized MIME type name complete with
    /// type/subtype, or simply the extension name such as
    /// "xml", "json", "png".
    ///
    /// These are all the same.
    ///
    /// ```
    /// ureq::post("/my_page")
    ///     .set("Content-Type", "text/plain")
    ///     .call();
    ///
    /// ureq::post("/my_page")
    ///     .content_type("text/plain")
    ///     .call();
    ///
    /// ureq::post("/my_page")
    ///     .content_type("txt")
    ///     .call();
    /// ```
    pub fn content_type<S>(&mut self, c: S) -> &mut Request
    where
        S: Into<String>,
    {
        self.set("Content-Type", mime_of(c))
    }

    /// Sets the `Accept` header in the same way as `content_type()`.
    ///
    /// The short-hand `.accept()` method accepts the
    /// canonicalized MIME type name complete with
    /// type/subtype, or simply the extension name such as
    /// "xml", "json", "png".
    ///
    /// These are all the same.
    ///
    /// ```
    /// ureq::get("/my_page")
    ///     .set("Accept", "text/plain")
    ///     .call();
    ///
    /// ureq::get("/my_page")
    ///     .accept("text/plain")
    ///     .call();
    ///
    /// ureq::get("/my_page")
    ///     .accept("txt")
    ///     .call();
    /// ```
    pub fn accept<S>(&mut self, accept: S) -> &mut Request
    where
        S: Into<String>,
    {
        self.set("Accept", mime_of(accept))
    }

    /// Timeout for the socket connection to be successful.
    ///
    /// The default is `0`, which means a request can block forever.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .timeout(1_000) // wait max 1 second to connect
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn timeout(&mut self, millis: u32) -> &mut Request {
        self.timeout = millis;
        self
    }

    /// Timeout for the individual reads of the socket.
    ///
    /// The default is `0`, which means it can block forever.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .timeout_read(1_000) // wait max 1 second for the read
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn timeout_read(&mut self, millis: u32) -> &mut Request {
        self.timeout_read = millis;
        self
    }

    /// Timeout for the individual writes to the socket.
    ///
    /// The default is `0`, which means it can block forever.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .timeout_write(1_000)   // wait max 1 second for sending.
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn timeout_write(&mut self, millis: u32) -> &mut Request {
        self.timeout_write = millis;
        self
    }

    /// Basic auth.
    ///
    /// These are the same
    ///
    /// ```
    /// let r1 = ureq::get("http://localhost/my_page")
    ///     .auth("martin", "rubbermashgum")
    ///     .call();
    ///  println!("{:?}", r1);
    ///
    /// let r2 = ureq::get("http://martin:rubbermashgum@localhost/my_page").call();
    /// println!("{:?}", r2);
    /// ```
    pub fn auth<S, T>(&mut self, user: S, pass: T) -> &mut Request
    where
        S: Into<String>,
        T: Into<String>,
    {
        let u = user.into();
        let p = pass.into();
        let pass = basic_auth(&u, &p);
        self.auth_kind("Basic", pass)
    }

    /// Auth of other kinds such as `Digest`, `Token` etc.
    ///
    /// ```
    /// let r = ureq::get("http://localhost/my_page")
    ///     .auth_kind("token", "secret")
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn auth_kind<S, T>(&mut self, kind: S, pass: T) -> &mut Request
    where
        S: Into<String>,
        T: Into<String>,
    {
        let s = format!("Authorization: {} {}", kind.into(), pass.into());
        let header = s.parse::<Header>().expect("Failed to parse header");
        add_header(header, &mut self.headers);
        self
    }

    /// How many redirects to follow.
    ///
    /// Defaults to `5`.
    ///
    /// ```
    /// let r = ureq::get("/my_page")
    ///     .redirects(10)
    ///     .call();
    /// println!("{:?}", r);
    /// ```
    pub fn redirects(&mut self, n: u32) -> &mut Request {
        self.redirects = n;
        self
    }

    // pub fn retry(&self, times: u16) -> Request {
    //     unimplemented!()
    // }
    // pub fn sortQuery(&self) -> Request {
    //     unimplemented!()
    // }
    // pub fn sortQueryBy(&self, by: Box<Fn(&str, &str) -> usize>) -> Request {
    //     unimplemented!()
    // }
    // pub fn ca<S>(&self, accept: S) -> Request
    //     where S: Into<String> {
    //     unimplemented!()
    // }
    // pub fn cert<S>(&self, accept: S) -> Request
    //     where S: Into<String> {
    //     unimplemented!()
    // }
    // pub fn key<S>(&self, accept: S) -> Request
    //     where S: Into<String> {
    //     unimplemented!()
    // }
    // pub fn pfx<S>(&self, accept: S) -> Request // TODO what type? u8?
    //     where S: Into<String> {
    //     unimplemented!()
    // }

    fn to_url(&self) -> Result<Url, Error> {
        URL_BASE
            .join(&self.path)
            .map_err(|e| Error::BadUrl(format!("{}", e)))
    }
}
