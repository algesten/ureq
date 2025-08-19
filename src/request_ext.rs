use crate::agent::AgentInstance;
use crate::config::typestate::RequestExtScope;
use crate::config::{Config, ConfigBuilder, RequestLevelConfig};
use crate::typestate::HttpCrateScope;
use crate::{http, Agent, AsSendBody, Body, Error};
use std::ops::Deref;
use ureq_proto::http::{Request, Response};

/// Extension trait for [`http::Request<impl AsSendBody>`].
///
/// Adds additional convenience methods to the `Request` that are not available
/// in the plain http API.
pub trait RequestExt<S>
where
    S: AsSendBody,
{
    /// Allows configuring the request behaviour, starting with the default [`Agent`].
    ///
    /// This method allows configuring the request by using the default Agent, and performing
    /// additional configurations on top.
    /// This method returns a `WithAgent` struct that it is possible to call `configure()` and `run()`
    /// on to configure the request behaviour, or run the request.
    ///
    /// # Example
    ///
    /// ```
    /// use ureq::{http, RequestExt, Error};
    ///
    /// let request: Result<http::Response<_>, Error> = http::Request::builder()
    ///             .method(http::Method::GET)
    ///             .uri("http://foo.bar")
    ///             .body(())
    ///             .unwrap()
    ///             .with_default_agent()
    ///             .configure()
    ///             .http_status_as_error(false)
    ///             .run();
    /// ```
    fn with_default_agent(self) -> WithAgent<'static, S>
    where
        Self: Sized,
    {
        let agent = Agent::new_with_defaults();
        Self::with_agent(self, agent)
    }

    /// Allows configuring this request behaviour, using a specific [`Agent`].
    ///
    /// This method allows configuring the request by using a user-provided `Agent` and performing
    /// additional configurations on top.
    /// This method returns a `WithAgent` struct that it is possible to call `configure()` and `run()`
    /// on to configure the request behaviour, or run the request.
    ///
    /// # Example
    ///
    /// ```
    /// use ureq::{http, Agent, RequestExt, Error};
    /// use std::time::Duration;
    /// let agent = Agent::config_builder()
    ///     .timeout_global(Some(Duration::from_secs(30)))
    ///     .build()
    ///     .new_agent();
    ///
    /// let request: Result<http::Response<_>, Error> = http::Request::builder()
    ///             .method(http::Method::GET)
    ///             .uri("http://foo.bar")
    ///             .body(())
    ///             .unwrap()
    ///             .with_agent(&agent)
    ///             .run();
    /// ```
    /// # Example with further customizations
    ///
    /// In this example we use a specific agent, but apply a request-specific configuration on top.
    ///
    /// ```
    /// use ureq::{http, Agent, RequestExt, Error};
    /// use std::time::Duration;
    /// let mut agent = Agent::config_builder()
    ///     .timeout_global(Some(Duration::from_secs(30)))
    ///     .build()
    ///     .new_agent();
    ///
    /// let request: Result<http::Response<_>, Error> = http::Request::builder()
    ///             .method(http::Method::GET)
    ///             .uri("http://foo.bar")
    ///             .body(())
    ///             .unwrap()
    ///             .with_agent(&agent)
    ///             .configure()
    ///             .http_status_as_error(false)
    ///             .run();
    /// ```
    fn with_agent<'a>(self, agent: impl Into<AgentRef<'a>>) -> WithAgent<'a, S>;

    /// Returns a [`ConfigBuilder`] for configuring the request.
    ///
    /// This is only to be used from within [`Middleware`](crate::middleware::Middleware).
    ///
    /// Returns `None` if not called from within a [`Middleware`](crate::middleware::Middleware).
    ///
    /// Any usage beyond from inside a middleware, synchronously with handling the request, is
    /// incorrect usage and might break without notice.
    fn middleware_config(self) -> Option<ConfigBuilder<HttpCrateScope<S>>>;
}

/// Wrapper struct that holds a [`Request`] associated with an [`Agent`].
pub struct WithAgent<'a, S: AsSendBody> {
    pub(crate) agent: AgentRef<'a>,
    pub(crate) request: Request<S>,
}

impl<'a, S: AsSendBody> WithAgent<'a, S> {
    /// Returns a [`ConfigBuilder`] for configuring the request.
    ///
    /// This allows setting additional request-specific options before sending the request.
    pub fn configure(self) -> ConfigBuilder<RequestExtScope<'a, S>> {
        ConfigBuilder(RequestExtScope(self))
    }

    /// Executes the request using the associated [`Agent`].
    pub fn run(self) -> Result<Response<Body>, Error> {
        self.agent.run(self.request)
    }
}

impl<'a, S: AsSendBody> WithAgent<'a, S> {
    pub(crate) fn request_level_config(&mut self) -> &mut Config {
        let request_level_config = self
            .request
            .extensions_mut()
            .get_mut::<RequestLevelConfig>();

        if request_level_config.is_none() {
            self.request
                .extensions_mut()
                .insert(self.agent.new_request_level_config());
        }

        // Unwrap is safe because of the above check
        let req_level: &mut RequestLevelConfig = self
            .request
            .extensions_mut()
            .get_mut::<RequestLevelConfig>()
            .unwrap();

        &mut req_level.0
    }
}

/// Reference type to hold an owned or borrowed [`Agent`].
pub enum AgentRef<'a> {
    Owned(Agent),
    Borrowed(&'a Agent),
}

impl<S: AsSendBody> RequestExt<S> for http::Request<S> {
    fn with_agent<'a>(self, agent: impl Into<AgentRef<'a>>) -> WithAgent<'a, S> {
        WithAgent {
            agent: agent.into(),
            request: self,
        }
    }

    fn middleware_config(self) -> Option<ConfigBuilder<HttpCrateScope<S>>> {
        let instance = self.extensions().get::<AgentInstance>()?.clone();
        Some(instance.0.configure_request(self))
    }
}

impl From<Agent> for AgentRef<'static> {
    fn from(value: Agent) -> Self {
        AgentRef::Owned(value)
    }
}

impl<'a> From<&'a Agent> for AgentRef<'a> {
    fn from(value: &'a Agent) -> Self {
        AgentRef::Borrowed(value)
    }
}

impl Deref for AgentRef<'_> {
    type Target = Agent;

    fn deref(&self) -> &Self::Target {
        match self {
            AgentRef::Owned(agent) => agent,
            AgentRef::Borrowed(agent) => agent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RequestLevelConfig;
    use crate::middleware::MiddlewareNext;
    use crate::SendBody;
    use std::time::Duration;

    #[test]
    fn configure_request_with_default_agent() {
        // Create `http` crate request and configure with trait
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://foo.bar")
            .body(())
            .unwrap()
            .with_default_agent()
            .configure()
            .https_only(true)
            .build();

        // Assert that the request-level configuration has been set
        let request_config = request
            .request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert!(request_config.0.https_only());
    }

    #[test]
    fn configure_request_default_agent_2() {
        // Create `http` crate request and configure with trait
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://foo.bar")
            .body(())
            .unwrap()
            .with_default_agent()
            .configure()
            .https_only(false)
            .build();

        // Assert that the request-level configuration has been set
        let request_config = request
            .request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert!(!request_config.0.https_only());
    }

    #[test]
    fn configure_request_default_agent_3() {
        // Create `http` crate request
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("http://foo.bar")
            .body("Some body")
            .unwrap();

        // Configure with the trait
        let request = request
            .with_default_agent()
            .configure()
            .http_status_as_error(true)
            .build();

        let request_config = request
            .request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert!(request_config.0.http_status_as_error());
    }

    #[test]
    fn configure_request_default_agent_4() {
        // Create `http` crate request
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("http://foo.bar")
            .body("Some body")
            .unwrap();

        // Configure with the trait
        let request = request
            .with_default_agent()
            .configure()
            .http_status_as_error(false)
            .build();

        let request_config = request
            .request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert!(!request_config.0.http_status_as_error());
    }

    #[test]
    fn configure_request_specified_agent() {
        // Create `http` crate request
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("http://foo.bar")
            .body("Some body")
            .unwrap();

        // Configure with the trait
        let agent = Agent::config_builder()
            .timeout_per_call(Some(Duration::from_secs(60)))
            .build()
            .new_agent();

        let request = request
            .with_agent(&agent)
            .configure()
            .http_status_as_error(false)
            .build();

        let request_config = request
            .request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        // The request-level config is the agent defaults + the explicitly configured stuff
        assert!(!request_config.0.http_status_as_error());
        assert_eq!(
            request_config.0.timeouts().per_call,
            Some(Duration::from_secs(60))
        );
    }

    #[test]
    fn middleware_config_none() {
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://foo.bar")
            .body(())
            .unwrap();

        assert!(request.middleware_config().is_none());
    }

    #[test]
    fn middleware_config() {
        fn my_middleware(
            req: Request<SendBody>,
            next: MiddlewareNext,
        ) -> Result<Response<Body>, crate::Error> {
            let config = req.middleware_config().unwrap();
            let req = config.build();
            let mut ret = next.handle(req)?;
            ret.extensions_mut().insert("worked".to_string());
            Ok(ret)
        }

        let agent = Agent::config_builder()
            .middleware(my_middleware)
            .build()
            .new_agent();

        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://httpbin.org/get")
            .body(())
            .unwrap();

        let request = request.with_agent(&agent);

        let ret = request.run().unwrap();
        assert_eq!(ret.extensions().get::<String>().unwrap(), "worked");
    }
}
