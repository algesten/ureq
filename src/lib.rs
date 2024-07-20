#![forbid(unsafe_code)]
#![warn(clippy::all)]
// #![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate log;

use std::convert::TryFrom;

/// Re-exported http-crate.
pub use http;

pub use body::{Body, BodyReader};
pub use config::AgentConfig;
use http::Method;
use http::{Request, Response, Uri};
pub use proxy::Proxy;
pub use request::RequestBuilder;
use request::{WithBody, WithoutBody};
pub use send_body::AsSendBody;

mod agent;
mod body;
mod config;
mod error;
mod pool;
mod proxy;
mod request;
mod send_body;
mod unit;
mod util;

pub mod resolver;
pub mod transport;

#[cfg(feature = "_tls")]
pub mod tls;

#[cfg(feature = "cookies")]
mod cookies;
#[cfg(feature = "cookies")]
pub use cookies::{Cookie, CookieJar};

pub use agent::Agent;
pub use error::{Error, TimeoutReason};
pub use send_body::SendBody;

/// Run a [`http::Request<impl AsSendBody>`].
pub fn run(request: Request<impl AsSendBody>) -> Result<Response<Body>, Error> {
    let agent = Agent::new_with_defaults();
    agent.run(request)
}

/// A new agent with default configuration
///
/// Agents are used to hold configuration and keep state between requests.
pub fn agent() -> Agent {
    Agent::new_with_defaults()
}

macro_rules! mk_method {
    ($f:tt, $m:tt, $b:ty) => {
        #[doc = concat!("Make a ", stringify!($m), " request.\n\nRun on a use-once [`Agent`].")]
        pub fn $f<T>(uri: T) -> RequestBuilder<$b>
        where
            Uri: TryFrom<T>,
            <Uri as TryFrom<T>>::Error: Into<http::Error>,
        {
            RequestBuilder::<$b>::new(Agent::new_with_defaults(), Method::$m, uri)
        }
    };
}

mk_method!(get, GET, WithoutBody);
mk_method!(post, POST, WithBody);
mk_method!(put, PUT, WithBody);
mk_method!(delete, DELETE, WithoutBody);
mk_method!(head, HEAD, WithoutBody);
mk_method!(options, OPTIONS, WithoutBody);
mk_method!(connect, CONNECT, WithoutBody);
mk_method!(patch, PATCH, WithBody);
mk_method!(trace, TRACE, WithoutBody);

#[cfg(test)]
pub(crate) mod test {

    use once_cell::sync::Lazy;

    use super::*;

    pub fn init_test_log() {
        static INIT_LOG: Lazy<()> = Lazy::new(|| env_logger::init());
        *INIT_LOG
    }

    #[test]
    fn connect_http_google() {
        init_test_log();
        let agent = Agent::new_with_defaults();

        let resp = agent.get("http://www.google.com/").call().unwrap();
        assert_eq!(
            "text/html;charset=ISO-8859-1",
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .replace("; ", ";")
        );
        assert_eq!(resp.body().mime_type(), Some("text/html"));
    }

    #[test]
    #[cfg(feature = "rustls")]
    fn connect_https_google_rustls() {
        init_test_log();
        use crate::tls::{TlsConfig, TlsProvider};

        let agent: Agent = AgentConfig {
            tls_config: TlsConfig {
                provider: TlsProvider::RustlsWithRing,
                ..Default::default()
            },
            ..Default::default()
        }
        .into();

        let resp = agent.get("https://www.google.com/").call().unwrap();
        assert_eq!(
            "text/html;charset=ISO-8859-1",
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .replace("; ", ";")
        );
        assert_eq!(resp.body().mime_type(), Some("text/html"));
    }

    #[test]
    #[cfg(feature = "native-tls")]
    fn connect_https_google_native_tls() {
        init_test_log();
        use crate::tls::{TlsConfig, TlsProvider};

        let agent: Agent = AgentConfig {
            tls_config: TlsConfig {
                provider: TlsProvider::NativeTls,
                ..Default::default()
            },
            ..Default::default()
        }
        .into();

        let mut resp = agent.get("https://www.google.com/").call().unwrap();

        assert_eq!(
            "text/html;charset=ISO-8859-1",
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .replace("; ", ";")
        );
        assert_eq!(resp.body().mime_type(), Some("text/html"));
        resp.body_mut().read_to_string(100_000).unwrap();
    }

    #[test]
    fn simple_put_content_len() {
        init_test_log();
        let mut resp = put("http://httpbin.org/put").send(&[0_u8; 100]).unwrap();
        resp.body_mut().read_to_string(1000).unwrap();
    }

    #[test]
    fn simple_put_chunked() {
        init_test_log();
        let mut resp = put("http://httpbin.org/put")
            // override default behavior
            .header("transfer-encoding", "chunked")
            .send(&[0_u8; 100])
            .unwrap();
        resp.body_mut().read_to_string(1000).unwrap();
    }

    #[test]
    fn connect_https_invalid_name() {
        let result = get("https://example.com{REQUEST_URI}/").call();
        let err = result.unwrap_err();
        assert!(matches!(err, Error::Http(_)));
        assert_eq!(err.to_string(), "http: invalid uri character");
    }

    // This doesn't need to run, just compile.
    fn _ensure_send_sync() {
        fn is_send(_t: impl Send) {}
        fn is_sync(_t: impl Sync) {}

        // Agent
        is_send(Agent::new_with_defaults());
        is_sync(Agent::new_with_defaults());

        // ResponseBuilder
        is_send(get("https://example.test"));
        is_sync(get("https://example.test"));

        let data = vec![0_u8, 1, 2, 3, 4];

        // Response<Body> via ResponseBuilder
        is_send(post("https://example.test").send(&data));
        is_sync(post("https://example.test").send(&data));

        // Request<impl AsBody>
        is_send(Request::post("https://yaz").body(&data).unwrap());
        is_sync(Request::post("https://yaz").body(&data).unwrap());

        // Response<Body> via Agent::run
        is_send(run(Request::post("https://yaz").body(&data).unwrap()));
        is_sync(run(Request::post("https://yaz").body(&data).unwrap()));

        // Response<BodyReader<'a>>
        let mut response = post("https://yaz").send(&data).unwrap();
        let shared_reader = response.body_mut().as_reader(1000);
        is_send(shared_reader);
        let shared_reader = response.body_mut().as_reader(1000);
        is_sync(shared_reader);

        // Response<BodyReader<'static>>
        let response = post("https://yaz").send(&data).unwrap();
        let owned_reader = response.into_parts().1.into_reader(1000);
        is_send(owned_reader);
        let response = post("https://yaz").send(&data).unwrap();
        let owned_reader = response.into_parts().1.into_reader(1000);
        is_sync(owned_reader);
    }
}
