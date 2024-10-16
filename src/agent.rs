use std::convert::TryFrom;
use std::fmt::Debug;
use std::sync::Arc;

use http::{Method, Request, Response, Uri};

use crate::body::Body;
use crate::config::{AgentScope, Config, ConfigBuilder, HttpCrateScope, RequestLevelConfig};
use crate::middleware::MiddlewareNext;
use crate::pool::ConnectionPool;
use crate::resolver::{DefaultResolver, Resolver};
use crate::send_body::AsSendBody;
use crate::transport::{Connector, DefaultConnector};
use crate::{Error, RequestBuilder, SendBody};
use crate::{WithBody, WithoutBody};

/// Agents keep state between requests.
///
/// By default, no state, such as cookies, is kept between requests.
/// But by creating an agent as entry point for the request, we
/// can keep a state.
///
/// # Example
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
/// # About threads and cloning
///
/// Agent uses inner [`Arc`]. Cloning an Agent results in an instance
/// that shares the same underlying connection pool and other state.
///
/// The connection pool contains an inner [`Mutex`][std::sync::Mutex] which is (briefly)
/// held when borrowing a pooled connection, or returning a connection to the pool.
///
/// All request functions in ureq have a signature similar to this:
///
/// ```
/// # use ureq::{Body, AsSendBody, Error};
/// fn run(request: http::Request<impl AsSendBody>) -> Result<http::Response<Body>, Error> {
///     // <something>
/// # todo!()
/// }
/// ```
///
/// It follows that:
///
/// * An Agent is borrowed for the duration of:
///     1. Sending the request header ([`http::Request`])
///     2. Sending the request body ([`SendBody`])
///     3. Receiving the response header ([`http::Response`])
/// * The [`Body`] of the response is not bound to the lifetime of the Agent.
///
/// A response [`Body`] can be streamed (for instance via [`Body::into_reader()`]). The [`Body`]
/// implements [`Send`], which means it's possible to read the response body on another thread than
/// the one that run the request. Behind the scenes, the [`Body`] retains the connection to the remote
/// server and it is returned to the agent's pool, once the Body instance (or reader) is dropped.
///
/// There is an asymmetry in that sending a request body will borrow the Agent instance, while receiving
/// the response body does not. This inconvencience is somewhat mitigated by that [`Agent::run()`] (or
/// going via the methods such as [`Agent::get()`]), borrows `&self`, i.e. not exclusive `mut` borrows.
///
/// That cloning the agent shares the connection pool is considered a feature. It is often useful to
/// retain a single pool for the entire process, while dispatching requests from different threads.
/// And if we want separate pools, we can create multiple agents via one of the constructors
/// (such as [`Agent::new_with_config()`]).
///
/// Note that both [`Config::clone()`] and [`Agent::clone()`] are  "cheap" meaning they should not
/// incur any heap allocation.
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

    /// Shortcut to reach a [`ConfigBuilder`]
    ///
    /// This is the same as doing [`Config::builder()`].
    pub fn config_builder() -> ConfigBuilder<AgentScope> {
        Config::builder()
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

    /// Access the shared cookie jar.
    ///
    /// Used to persist and manipulate the cookies. The jar is shared between
    /// all clones of the same [`Agent`], meaning you must drop the CookieJar
    /// before using the agent, or end up with a deadlock.
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
    /// let jar = agent.cookie_jar_lock();
    ///
    /// jar.save_json(&mut file)?;
    ///
    /// // Release the cookie jar to use agents again.
    /// jar.release();
    ///
    /// # Ok::<_, ureq::Error>(())
    /// ```
    #[cfg(feature = "cookies")]
    pub fn cookie_jar_lock(&self) -> crate::cookies::CookieJar<'_> {
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
    pub fn configure_request<S: AsSendBody>(
        &self,
        mut request: Request<S>,
    ) -> ConfigBuilder<HttpCrateScope<S>> {
        let exts = request.extensions_mut();

        if exts.get::<RequestLevelConfig>().is_none() {
            exts.insert(self.new_request_level_config());
        }

        ConfigBuilder(HttpCrateScope(request))
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
impl Agent {
    pub fn pool_count(&self) -> usize {
        self.pool.pool_count()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use assert_no_alloc::*;

    #[test]
    fn agent_clone_does_not_allocate() {
        let a = Agent::new_with_defaults();
        assert_no_alloc(|| a.clone());
    }
}
