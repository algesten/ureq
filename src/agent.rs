use std::convert::TryFrom;
use std::fmt::Debug;
use std::sync::Arc;

use http::{Method, Request, Response, Uri};

use crate::body::Body;
use crate::config::RequestLevelConfig;
use crate::middleware::MiddlewareNext;
use crate::pool::ConnectionPool;
use crate::resolver::{DefaultResolver, Resolver};
use crate::send_body::AsSendBody;
use crate::transport::{Connector, DefaultConnector};
use crate::{Config, Error, RequestBuilder, SendBody};
use crate::{WithBody, WithoutBody};

/// Agents keep state between requests.
///
/// By default, no state, such as cookies, is kept between requests.
/// But by creating an agent as entry point for the request, we
/// can keep a state.
///
/// ```no_run
/// let mut agent = ureq::agent();
///
/// agent
///     .post("http://example.com/post/login")
///     .send(b"my password")?;
///
/// let secret = agent
///     .get("http://example.com/get/my-protected-page")
///     .call()?
///     .body_mut()
///     .read_to_string()?;
///
///   println!("Secret is: {}", secret);
/// # Ok::<_, ureq::Error>(())
/// ```
///
/// Agent uses inner `Arc`, so cloning an Agent results in an instance
/// that shares the same underlying connection pool and other state.
#[derive(Debug, Clone)]
pub struct Agent {
    pub(crate) config: Arc<Config>,
    pub(crate) pool: Arc<ConnectionPool>,
    pub(crate) resolver: Arc<dyn Resolver>,

    #[cfg(feature = "cookies")]
    pub(crate) jar: Arc<crate::cookies::SharedCookieJar>,
}

impl Agent {
    /// Creates an agent with defaults.
    pub fn new_with_defaults() -> Self {
        Self::with_parts(
            Config::default(),
            DefaultConnector::default(),
            DefaultResolver::default(),
        )
    }

    /// Creates an agent with config.
    pub fn new_with_config(config: Config) -> Self {
        Self::with_parts(
            config,
            DefaultConnector::default(),
            DefaultResolver::default(),
        )
    }

    /// Creates an agent with a bespoke transport and resolver.
    ///
    /// _This is low level API that isn't for regular use of ureq._
    pub fn with_parts(config: Config, connector: impl Connector, resolver: impl Resolver) -> Self {
        let pool = Arc::new(ConnectionPool::new(connector, &config));

        Agent {
            config: Arc::new(config),
            pool,
            resolver: Arc::new(resolver),

            #[cfg(feature = "cookies")]
            jar: Arc::new(crate::cookies::SharedCookieJar::new()),
        }
    }

    /// Access the cookie jar.
    ///
    /// Used to persist and manipulate the cookies.
    ///
    /// ```no_run
    /// use std::io::Write;
    /// use std::fs::File;
    ///
    /// let agent = ureq::agent();
    ///
    /// // Cookies set by www.google.com are stored in agent.
    /// agent.get("https://www.google.com/").call()?;
    ///
    /// // Saves (persistent) cookies
    /// let mut file = File::create("cookies.json")?;
    /// agent.cookie_jar().save_json(&mut file)?;
    /// # Ok::<_, ureq::Error>(())
    /// ```
    #[cfg(feature = "cookies")]
    pub fn cookie_jar(&self) -> crate::cookies::CookieJar<'_> {
        self.jar.lock()
    }

    /// Run a [`http::Request<impl AsSendBody>`].
    ///
    /// Used to execute http crate [`http::Request`] directly on this agent.
    ///
    /// # Example
    ///
    /// ```
    /// use ureq::Agent;
    ///
    /// let agent: Agent = Agent::new_with_defaults();
    ///
    /// let mut request =
    ///     http::Request::get("http://httpbin.org/get")
    ///     .body(())?;
    ///
    /// let body = agent.run(request)?
    ///     .body_mut()
    ///     .read_to_string()?;
    /// # Ok::<(), ureq::Error>(())
    /// ```
    pub fn run(&self, request: Request<impl AsSendBody>) -> Result<Response<Body>, Error> {
        let (parts, mut body) = request.into_parts();
        let body = body.as_body();
        let request = Request::from_parts(parts, ());

        self.run_via_middleware(request, body)
    }

    pub(crate) fn run_via_middleware(
        &self,
        request: Request<()>,
        body: SendBody,
    ) -> Result<Response<Body>, Error> {
        let (parts, _) = request.into_parts();
        let request = http::Request::from_parts(parts, body);

        let next = MiddlewareNext::new(self);
        next.handle(request)
    }

    /// Get the config for this agent.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Alter the configuration for an http crate request.
    ///
    /// Notice: It's an error to configure a [`http::Request`] using
    /// one instance of [`Agent`] and run using another instance. The
    /// library does not currently detect this situation, but it is
    /// not considered a breaking change if this is enforced in
    /// the future.
    pub fn configure_request<'a>(
        &self,
        request: &'a mut Request<impl AsSendBody + 'static>,
    ) -> &'a mut Config {
        let exts = request.extensions_mut();

        if exts.get::<RequestLevelConfig>().is_none() {
            exts.insert(self.new_request_level_config());
        }

        // Unwrap is OK because of above check
        let req_level: &mut RequestLevelConfig = exts.get_mut().unwrap();

        &mut req_level.0
    }

    pub(crate) fn new_request_level_config(&self) -> RequestLevelConfig {
        RequestLevelConfig(self.config.as_ref().clone())
    }
}

macro_rules! mk_method {
    ($(($f:tt, $m:tt, $b:ty)),*) => {
        impl Agent {
            $(
                #[doc = concat!("Make a ", stringify!($m), " request using this agent.")]
                #[must_use]
                pub fn $f<T>(&self, uri: T) -> RequestBuilder<$b>
                where
                    Uri: TryFrom<T>,
                    <Uri as TryFrom<T>>::Error: Into<http::Error>,
                {
                    RequestBuilder::<$b>::new(self.clone(), Method::$m, uri)
                }
            )*
        }
    };
}

mk_method!(
    (get, GET, WithoutBody),
    (post, POST, WithBody),
    (put, PUT, WithBody),
    (delete, DELETE, WithoutBody),
    (head, HEAD, WithoutBody),
    (options, OPTIONS, WithoutBody),
    (connect, CONNECT, WithoutBody),
    (patch, PATCH, WithBody),
    (trace, TRACE, WithoutBody)
);

impl From<Config> for Agent {
    fn from(value: Config) -> Self {
        Agent::new_with_config(value)
    }
}

#[cfg(test)]
impl crate::Agent {
    pub fn pool_count(&self) -> usize {
        self.pool.pool_count()
    }
}
