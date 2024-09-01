use std::convert::TryFrom;
use std::fmt::Debug;
use std::sync::Arc;

use http::{Method, Request, Response, Uri};

use crate::body::Body;
use crate::middleware::MiddlewareNext;
use crate::pool::ConnectionPool;
use crate::resolver::{DefaultResolver, Resolver};
use crate::run::run;
use crate::send_body::AsSendBody;
use crate::transport::{Connector, DefaultConnector};
use crate::{AgentConfig, Error, RequestBuilder, SendBody};
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
    pub(crate) config: Arc<AgentConfig>,
    pub(crate) pool: Arc<ConnectionPool>,
    pub(crate) resolver: Arc<dyn Resolver>,

    #[cfg(feature = "cookies")]
    pub(crate) jar: Arc<crate::cookies::SharedCookieJar>,
}

impl Agent {
    /// Creates an agent with defaults.
    pub fn new_with_defaults() -> Self {
        Self::with_parts(
            AgentConfig::default(),
            DefaultConnector::default(),
            DefaultResolver::default(),
        )
    }

    /// Creates an agent with config.
    pub fn new_with_config(config: AgentConfig) -> Self {
        Self::with_parts(
            config,
            DefaultConnector::default(),
            DefaultResolver::default(),
        )
    }

    /// Creates an agent with a bespoke transport and resolver.
    ///
    /// _This is low level API that isn't for regular use of ureq._
    pub fn with_parts(
        config: AgentConfig,
        connector: impl Connector,
        resolver: impl Resolver,
    ) -> Self {
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
    pub fn run(&self, request: Request<impl AsSendBody>) -> Result<Response<Body>, Error> {
        let (parts, mut body) = request.into_parts();
        let body = body.as_body();
        let request = Request::from_parts(parts, ());

        self.do_run(request, body)
    }

    pub(crate) fn run_middleware(
        &self,
        request: Request<()>,
        body: SendBody,
    ) -> Result<Response<Body>, Error> {
        let (parts, _) = request.into_parts();
        let request = http::Request::from_parts(parts, body);

        let next = MiddlewareNext::new(self);
        next.handle(request)
    }

    pub(crate) fn do_run(
        &self,
        request: Request<()>,
        body: SendBody,
    ) -> Result<Response<Body>, Error> {
        run(self, request, body)
    }

    /// Get the config for this agent.
    pub fn config(&self) -> &AgentConfig {
        &self.config
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

impl From<AgentConfig> for Agent {
    fn from(value: AgentConfig) -> Self {
        Agent::new_with_config(value)
    }
}

#[cfg(test)]
impl crate::Agent {
    pub fn pool_count(&self) -> usize {
        self.pool.pool_count()
    }
}
