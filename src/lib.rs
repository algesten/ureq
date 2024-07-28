#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![deny(missing_docs)]
//!<div align="center">
//!  <!-- Version -->
//!  <a href="https://crates.io/crates/ureq">
//!    <img src="https://img.shields.io/crates/v/ureq.svg?style=flat-square"
//!    alt="Crates.io version" />
//!  </a>
//!  <!-- Docs -->
//!  <a href="https://docs.rs/ureq">
//!    <img src="https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square"
//!      alt="docs.rs docs" />
//!  </a>
//!  <!-- Downloads -->
//!  <a href="https://crates.io/crates/ureq">
//!    <img src="https://img.shields.io/crates/d/ureq.svg?style=flat-square"
//!      alt="Crates.io downloads" />
//!  </a>
//!</div>
//!
//! A simple, safe HTTP client.
//!
//! Ureq's first priority is being easy for you to use. It's great for
//! anyone who wants a low-overhead HTTP client that just gets the job done. Works
//! very well with HTTP APIs. Its features include cookies, JSON, HTTP proxies,
//! HTTPS, interoperability with the `http` crate, and charset decoding.
//!
//! Ureq is in pure Rust for safety and ease of understanding. It avoids using
//! `unsafe` directly. It [uses blocking I/O][blocking] instead of async I/O, because that keeps
//! the API simple and keeps dependencies to a minimum. For TLS, ureq uses
//! [rustls or native-tls](#https--tls--ssl).
//!
//! See the [changelog] for details of recent releases.
//!
//! [blocking]: #blocking-io-for-simplicity
//! [changelog]: https://github.com/algesten/ureq/blob/main/CHANGELOG.md
//!
//! ## Usage
//!
//! In its simplest form, ureq looks like this:
//!
//! ```rust
//! let body: String = ureq::get("http://example.com")
//!     .header("Example-Header", "header value")
//!     .call()?
//!     .body_mut()
//!     .read_to_string()?;
//! # Ok::<(), ureq::Error>(())
//! ```
//!
//! For more involved tasks, you'll want to create an [Agent]. An Agent
//! holds a connection pool for reuse, and a cookie store if you use the
//! **cookies** feature. An Agent can be cheaply cloned due to internal
//! [Arc](std::sync::Arc) and all clones of an Agent share state among each other. Creating
//! an Agent also allows setting options like the TLS configuration.
//!
//! ```no_run
//! use ureq::{Agent, AgentConfig};
//! use std::time::Duration;
//!
//! let agent: Agent = AgentConfig {
//!     timeout_global: Some(Duration::from_secs(5)),
//!     ..Default::default()
//! }.into();
//!
//! let body: String = agent.get("http://example.com/page")
//!     .call()?
//!     .body_mut()
//!     .read_to_string()?;
//!
//! // Reuses the connection from previous request.
//! let response: String = agent.put("http://example.com/upload")
//!     .header("Authorization", "example-token")
//!     .send("some body data")?
//!     .body_mut()
//!     .read_to_string()?;
//! # Ok::<_, ureq::Error>(())
//! ```
//!
//! Ureq supports sending and receiving json, if you enable the **json** feature:
//!
//! ```no_run
//! # #[cfg(feature = "json")]
//! # {
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Serialize)]
//! struct MySendBody {
//!    thing: String,
//! }
//!
//! #[derive(Deserialize)]
//! struct MyRecvBody {
//!    other: String,
//! }
//!
//! let send_body = MySendBody { thing: "yo".to_string() };
//!
//! // Requires the `json` feature enabled.
//! let recv_body = ureq::post("http://example.com/post/ingest")
//!     .header("X-My-Header", "Secret")
//!     .send_json(&send_body)?
//!     .body_mut()
//!     .read_json::<MyRecvBody>()?;
//! # }
//! # Ok::<_, ureq::Error>(())
//! ```

#[macro_use]
extern crate log;

use std::convert::TryFrom;

/// Re-exported http-crate.
pub use http;

pub use body::{Body, BodyReader, BodyReaderConfig};
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

/// A new [Agent] with default configuration
///
/// Agents are used to hold configuration and keep state between requests.
pub fn agent() -> Agent {
    Agent::new_with_defaults()
}

macro_rules! mk_method {
    ($f:tt, $m:tt, $b:ty) => {
        #[doc = concat!("Make a ", stringify!($m), " request.\n\nRun on a use-once [`Agent`].")]
        #[must_use]
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

        let res = agent.get("http://www.google.com/").call().unwrap();
        assert_eq!(
            "text/html;charset=ISO-8859-1",
            res.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .replace("; ", ";")
        );
        assert_eq!(res.body().mime_type(), Some("text/html"));
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

        let res = agent.get("https://www.google.com/").call().unwrap();
        assert_eq!(
            "text/html;charset=ISO-8859-1",
            res.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .replace("; ", ";")
        );
        assert_eq!(res.body().mime_type(), Some("text/html"));
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

        let mut res = agent.get("https://www.google.com/").call().unwrap();

        assert_eq!(
            "text/html;charset=ISO-8859-1",
            res.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .replace("; ", ";")
        );
        assert_eq!(res.body().mime_type(), Some("text/html"));
        res.body_mut().read_to_string().unwrap();
    }

    #[test]
    fn simple_put_content_len() {
        init_test_log();
        let mut res = put("http://httpbin.org/put").send(&[0_u8; 100]).unwrap();
        res.body_mut().read_to_string().unwrap();
    }

    #[test]
    fn simple_put_chunked() {
        init_test_log();
        let mut res = put("http://httpbin.org/put")
            // override default behavior
            .header("transfer-encoding", "chunked")
            .send(&[0_u8; 100])
            .unwrap();
        res.body_mut().read_to_string().unwrap();
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
        let shared_reader = response.body_mut().as_reader();
        is_send(shared_reader);
        let shared_reader = response.body_mut().as_reader();
        is_sync(shared_reader);

        // Response<BodyReader<'static>>
        let response = post("https://yaz").send(&data).unwrap();
        let owned_reader = response.into_parts().1.into_reader();
        is_send(owned_reader);
        let response = post("https://yaz").send(&data).unwrap();
        let owned_reader = response.into_parts().1.into_reader();
        is_sync(owned_reader);
    }
}

// TODO(martin): JSON send/receive bodies
// TODO(martin): retry idemptotent methods
