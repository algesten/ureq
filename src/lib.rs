#![forbid(unsafe_code)]
#![warn(clippy::all)]
// #![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use std::convert::TryFrom;
use std::sync::atomic::{AtomicBool, Ordering};

use body::RecvBody;
/// Re-exported http-crate.
pub use http;

use http::{Method, Request, Response, Uri};
use once_cell::sync::Lazy;
use request::RequestBuilder;

mod agent;
mod body;
mod error;
mod flow;
mod pool;
mod request;
mod time;
mod transport;
mod unit;

pub use agent::{Agent, AgentConfig};
pub use body::Body;
pub use error::Error;

pub fn run(request: &Request<impl Body>) -> Result<Response<RecvBody>, Error> {
    let mut agent = Agent::new_default();
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

// is_test returns false so long as it has only ever been called with false.
// If it has ever been called with true, it will always return true after that.
// This is a public but hidden function used to allow doctests to use the test_agent.
// Note that we use this approach for doctests rather the #[cfg(test)], because
// doctests are run against a copy of the crate build without cfg(test) set.
// We also can't use #[cfg(doctest)] to do this, because cfg(doctest) is only set
// when collecting doctests, not when building the crate.
#[doc(hidden)]
pub fn is_test(is: bool) -> bool {
    static IS_TEST: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
    if is {
        IS_TEST.store(true, Ordering::SeqCst);
    }
    IS_TEST.load(Ordering::SeqCst)
}
