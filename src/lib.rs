#![forbid(unsafe_code)]
#![warn(clippy::all)]
// #![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate log;

use std::convert::TryFrom;

use body::AsBody;
/// Re-exported http-crate.
pub use http;

use http::Method;
pub use http::{Request, Response, Uri};
use recv::RecvBody;
pub use request::RequestBuilder;

mod agent;
mod body;
mod error;
mod pool;
mod proxy;
mod recv;
mod request;
pub mod resolver;
mod time;
pub mod transport;
mod unit;
mod util;

#[cfg(feature = "tls")]
pub mod tls;

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
    use std::io::Read;

    use super::*;

    #[test]
    fn simple_get() {
        env_logger::init();
        let mut response = get("https://www.lookback.com/").call().unwrap();
        // println!("{:#?}", response);
        let mut body = String::new();
        response.body_mut().read_to_string(&mut body).unwrap();
        // println!("body: {:?}", body);
    }

    // This doesn't need to run, just compile.
    fn _ensure_send_sync() {
        fn is_send(_t: impl Send) {}
        fn is_sync(_t: impl Sync) {}

        // Agent
        is_send(Agent::new_default());
        is_sync(Agent::new_default());

        // ResponseBuilder
        is_send(get("https://example.test"));
        is_sync(get("https://example.test"));

        let data = vec![0_u8, 1, 2, 3, 4];

        // Response<RecvBody> via ResponseBuilder
        is_send(post("https://example.test").send_bytes(&data));
        is_sync(post("https://example.test").send_bytes(&data));

        // Request<impl AsBody>
        is_send(Request::post("https://yaz").body(&data).unwrap());
        is_sync(Request::post("https://yaz").body(&data).unwrap());

        // Response<RecvBody> via Agent::run
        is_send(run(Request::post("https://yaz").body(&data).unwrap()));
        is_sync(run(Request::post("https://yaz").body(&data).unwrap()));
    }
}
