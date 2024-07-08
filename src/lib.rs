#![forbid(unsafe_code)]
#![warn(clippy::all)]
// #![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use std::convert::TryFrom;

use body::AsBody;
/// Re-exported http-crate.
pub use http;

use http::{Method, Request, Response, Uri};
use recv::RecvBody;
use request::RequestBuilder;

mod agent;
mod body;
mod error;
mod pool;
mod recv;
mod request;
pub mod resolver;
mod time;
pub mod transport;
mod unit;

pub use agent::{Agent, AgentConfig};
pub use body::Body;
pub use error::Error;

pub fn run(request: Request<impl AsBody>) -> Result<Response<RecvBody>, Error> {
    let agent = Agent::new_default();
    agent.run(request)
}

fn builder<T>(method: Method, uri: T) -> RequestBuilder
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    let agent = Agent::new_default();
    RequestBuilder::new(agent, method, uri)
}

/// Make a GET request.
pub fn get<T>(uri: T) -> RequestBuilder
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    builder(Method::GET, uri)
}

/// Make a POST request.
pub fn post<T>(uri: T) -> RequestBuilder
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    builder(Method::POST, uri)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn simple_get() {
        get("https://httpbin.org/get").call().unwrap();
    }
}
