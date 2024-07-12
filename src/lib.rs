#![forbid(unsafe_code)]
#![warn(clippy::all)]
// #![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate log;

use std::convert::TryFrom;

/// Re-exported http-crate.
pub use http;
use send_body::AsBody;

pub use body::Body;
use http::Method;
pub use http::{Request, Response, Uri};
pub use request::RequestBuilder;

mod agent;
mod body;
mod error;
mod pool;
mod proxy;
mod request;
mod send_body;
mod time;
mod unit;
mod util;

pub mod resolver;
pub mod transport;

#[cfg(feature = "tls")]
pub mod tls;

#[cfg(feature = "cookies")]
mod cookies;
#[cfg(feature = "cookies")]
pub use cookies::CookieJar;

pub use agent::{Agent, AgentConfig};
pub use error::Error;
pub use send_body::SendBody;

/// Run a [`http::Request`]
pub fn run(request: Request<impl AsBody>) -> Result<Response<Body>, Error> {
    let agent = Agent::new_default();
    agent.run(request)
}

/// A new agent with default configuration
///
/// Agents are used to hold configuration and keep state between requests.
pub fn agent() -> Agent {
    Agent::new_default()
}

macro_rules! mk_method {
    ($f:tt, $m:tt) => {
        #[doc = concat!("Make a ", stringify!($m), " request")]
        pub fn $f<T>(uri: T) -> RequestBuilder
        where
            Uri: TryFrom<T>,
            <Uri as TryFrom<T>>::Error: Into<http::Error>,
        {
            RequestBuilder::new(Agent::new_default(), Method::$m, uri)
        }
    };
}

mk_method!(get, GET);
mk_method!(post, POST);
mk_method!(put, PUT);
mk_method!(delete, DELETE);
mk_method!(head, HEAD);
mk_method!(options, OPTIONS);
mk_method!(connect, CONNECT);
mk_method!(patch, PATCH);
mk_method!(trace, TRACE);

#[cfg(test)]
mod test {
    use std::io::Read;

    use super::*;

    #[test]
    fn simple_get() {
        env_logger::init();
        let mut response = get("https://httpbin.org/relative-redirect/3")
            .call()
            .unwrap();
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
