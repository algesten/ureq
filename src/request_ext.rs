use std::ops::{Deref, DerefMut};
use ureq_proto::http::{Request, Response};
use crate::config::{Config, ConfigBuilder, RequestLevelConfig};
use crate::typestate::{HttpCrateScope, RequestScope};
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
    /// TODO
    ///
    /// # Example
    ///
    /// ```
    /// use ureq::{http, RequestExt};
    ///
    /// http::Request::builder()
    ///     .method(http::Method::POST)
    ///     .uri("http://httpbin.org/post")
    ///     .body(())
    ///     .unwrap()
    ///     .with_default_agent()
    ///     .run();
    /// ```
    fn with_default_agent(self) -> WithAgent<'static, S> where Self: Sized {
        let agent = Agent::new_with_defaults();
        Self::with_agent(self, agent)
    }

    /// Use this [`Request`] with a ureq [`Agent`].
    ///
    /// # Example
    ///
    /// ```
    /// use ureq::{http, Agent, RequestExt};
    /// use std::time::Duration;
    ///
    /// let request: http::Request<()> = http::Request::builder()
    ///     .method(http::Method::GET)
    ///     .uri("http://httpbin.org/get")
    ///     .body(())
    ///     .unwrap();
    ///
    /// let mut agent = Agent::config_builder()
    ///     .timeout_global(Some(Duration::from_secs(30)))
    ///     .build()
    ///     .new_agent();
    ///
    /// let response = request
    ///     .with_agent(&mut agent)
    ///     .configure()
    ///     .https_only(true)
    ///     .run()
    ///     .unwrap();
    /// ```
    fn with_agent<'a>(self, agent: impl Into<AgentRef<'a>>) -> WithAgent<'a, S>;
}

pub struct WithAgent<'a, S: AsSendBody> {
    pub(crate) agent: AgentRef<'a>,
    pub(crate) request: Request<S>,
}

impl<'a, S: AsSendBody> WithAgent<'a, S> {
    /// TODO
    pub fn configure(mut self) -> ConfigBuilder<RequestExtScope<'a, S>> {
        let exts = self.request.extensions_mut();

        if exts.get::<RequestLevelConfig>().is_none() {
            exts.insert(self.agent.new_request_level_config());
        }

        ConfigBuilder(RequestExtScope(self))
    }

    /// TODO
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

        // Unwrap is OK because of above check
        let req_level: &mut RequestLevelConfig = self.request.extensions_mut().get_mut::<RequestLevelConfig>().unwrap();

        &mut req_level.0
    }
}

/// Glue type to hold an owned or &mut Agent.
pub enum AgentRef<'a> {
    /// TODO
    Owned(Agent),
    /// TODO
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
        // Create `http` crate request
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://foo.bar")
            .body(())
            .unwrap()
            .with_default_agent()
            .configure()
            .http_status_as_error(true)
            .build();

        // Configure with the trait
        // let request = request.configure().https_only(true).build();

        dbg!(&request.request.extensions().get::<RequestLevelConfig>());
        dbg!(&request.agent.deref());

        // let request_config = request
        //     .extensions()
        //     .get::<RequestLevelConfig>()
        //     .cloned()
        //     .unwrap();
        //
        // assert_eq!(request_config.0.https_only(), true);

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
