use std::ops::{Deref, DerefMut};
use ureq_proto::http::{Request, Response};
use crate::config::{Config, ConfigBuilder, RequestLevelConfig};
use crate::{http, Agent, AsSendBody, Body, Error};
use crate::config::typestate::RequestExtScope;

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
    fn with_default_agent(self) -> WithAgent<'static, S> where Self: Sized {
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
    ///             .with_agent(agent)
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
    ///             .with_agent(agent)
    ///             .configure()
    ///             .http_status_as_error(false)
    ///             .run();
    /// ```
    fn with_agent<'a>(self, agent: impl Into<AgentRef<'a>>) -> WithAgent<'a, S>;
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
        let request_level_config = self.request
            .extensions_mut()
            .get_mut::<RequestLevelConfig>();

        if request_level_config.is_none() {
            self.request.extensions_mut().insert(self.agent.new_request_level_config());
        }

        // Unwrap is safe because of the above check
        let req_level: &mut RequestLevelConfig = self.request.extensions_mut().get_mut::<RequestLevelConfig>().unwrap();

        &mut req_level.0
    }
}

/// Reference type to hold an owned or borrowed [`Agent`].
pub enum AgentRef<'a> {
    Owned(Agent),
    Borrowed(&'a mut Agent),
}


impl<S: AsSendBody> RequestExt<S> for http::Request<S> {
    fn with_agent<'a>(self, agent: impl Into<AgentRef<'a>>) -> WithAgent<'a, S> {
        WithAgent {
            agent: agent.into(),
            request: self,
        }
    }
}


impl From<Agent> for AgentRef<'static> {
    fn from(value: Agent) -> Self {
        AgentRef::Owned(value)
    }
}

impl<'a> From<&'a mut Agent> for AgentRef<'a> {
    fn from(value: &'a mut Agent) -> Self {
        AgentRef::Borrowed(value)
    }
}

impl Deref for AgentRef<'_> {
    type Target = Agent;

    fn deref(&self) -> &Self::Target {
        match self {
            AgentRef::Owned(agent) => agent,
            AgentRef::Borrowed(agent) => &*agent,
        }
    }
}

impl DerefMut for AgentRef<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
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

    #[test]
    fn set_https_only_to_true_on_get_request() {
        // Create `http` crate request and configure with trait
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://foo.bar")
            .body(())
            .unwrap()
            .with_default_agent()
            .configure()
            .http_status_as_error(false)
            .build();

        // Assert that the request-level configuration has been set
        let request_config = request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert_eq!(request_config.0.https_only(), true);

        todo!();
    }

    // #[test]
    // fn set_https_only_to_false_on_get_request() {
    //     // Create `http` crate request
    //     let request = http::Request::builder()
    //         .method(http::Method::GET)
    //         .uri("http://foo.bar")
    //         .body(())
    //         .unwrap();
    //
    //     // Configure with the trait
    //     let request = request.configure().https_only(false).build();
    //
    //     let request_config = request
    //         .extensions()
    //         .get::<RequestLevelConfig>()
    //         .cloned()
    //         .unwrap();
    //
    //     assert_eq!(request_config.0.https_only(), false);
    // }
    //
    // #[test]
    // fn set_http_status_as_error_to_true_on_post_request() {
    //     // Create `http` crate request
    //     let request = http::Request::builder()
    //         .method(http::Method::POST)
    //         .uri("http://foo.bar")
    //         .body("Some body")
    //         .unwrap();
    //
    //     // Configure with the trait
    //     let request = request.configure().http_status_as_error(true).build();
    //
    //     let request_config = request
    //         .extensions()
    //         .get::<RequestLevelConfig>()
    //         .cloned()
    //         .unwrap();
    //
    //     assert_eq!(request_config.0.http_status_as_error(), true);
    // }
    //
    // #[test]
    // fn set_http_status_as_error_to_false_on_post_request() {
    //     // Create `http` crate request
    //     let request = http::Request::builder()
    //         .method(http::Method::GET)
    //         .uri("http://foo.bar")
    //         .body("Some body")
    //         .unwrap();
    //
    //     // Configure with the trait
    //     let request = request.configure().http_status_as_error(false).build();
    //
    //     let request_config = request
    //         .extensions()
    //         .get::<RequestLevelConfig>()
    //         .cloned()
    //         .unwrap();
    //
    //     assert_eq!(request_config.0.http_status_as_error(), false);
    // }
}
