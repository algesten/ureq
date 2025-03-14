use std::ops::{Deref, DerefMut};

use ureq_proto::http::{Request, Response};

use crate::config::typestate::RequestExtScope;
use crate::config::ConfigBuilder;
use crate::{Agent, AsSendBody, Body, Error};

/// TODO
pub trait RequestExt<S: AsSendBody>: Sized {
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
    fn with_default_agent(self) -> WithAgent<'static, S> {
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

impl<S: AsSendBody> RequestExt<S> for Request<S> {
    fn with_agent<'a>(self, agent: impl Into<AgentRef<'a>>) -> WithAgent<'a, S> {
        WithAgent {
            agent: agent.into(),
            request: self,
        }
    }
}

/// TODO
pub struct WithAgent<'a, S: AsSendBody> {
    agent: AgentRef<'a>,
    request: Request<S>,
}

impl<'a, S: AsSendBody> WithAgent<'a, S> {
    /// TODO
    pub fn configure(self) -> ConfigBuilder<RequestExtScope<'a, S>> {
        todo!()
    }

    /// TODO
    pub fn run(self) -> Result<Response<Body>, Error> {
        self.agent.run(self.request)
    }
}

/// Glue type to hold an owned or &mut Agent.
pub enum AgentRef<'a> {
    /// TODO
    Owned(Agent),
    /// TODO
    Borrowed(&'a mut Agent),
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
