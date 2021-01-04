#![forbid(unsafe_code)]
#![warn(clippy::all)]
//! A simple, safe HTTP client.
//!
//! Ureq's first priority is being easy for you to use. It's great for
//! anyone who wants a low-overhead HTTP client that just gets the job done. Works
//! very well with HTTP APIs. Its features include cookies, JSON, HTTP proxies,
//! HTTPS, and charset decoding.
//!
//! Ureq is in pure Rust for safety and ease of understanding. It avoids using
//! `unsafe` directly. It [uses blocking I/O][blocking] instead of async I/O, because that keeps
//! the API simple and and keeps dependencies to a minimum. For TLS, ureq uses
//! [rustls].
//!
//! Version 2.0.0 was released recently and changed some APIs. See the [changelog] for details.
//!
//! [blocking]: #blocking-io-for-simplicity
//! [changelog]: https://github.com/algesten/ureq/blob/master/CHANGELOG.md
//!
//!
//! ## Usage
//!
//! In its simplest form, ureq looks like this:
//!
//! ```rust
//! fn main() -> Result<(), ureq::Error> {
//! # ureq::is_test(true);
//!     let body: String = ureq::get("http://example.com")
//!         .set("Example-Header", "header value")
//!         .call()?
//!         .into_string()?;
//!     Ok(())
//! }
//! ```
//!
//! For more involved tasks, you'll want to create an [Agent]. An Agent
//! holds a connection pool for reuse, and a cookie store if you use the
//! "cookies" feature. An Agent can be cheaply cloned due to an internal
//! [Arc](std::sync::Arc) and all clones of an Agent share state among each other. Creating
//! an Agent also allows setting options like the TLS configuration.
//!
//! ```no_run
//! # fn main() -> std::result::Result<(), ureq::Error> {
//! # ureq::is_test(true);
//!   use ureq::{Agent, AgentBuilder};
//!   use std::time::Duration;
//!
//!   let agent: Agent = ureq::AgentBuilder::new()
//!       .timeout_read(Duration::from_secs(5))
//!       .timeout_write(Duration::from_secs(5))
//!       .build();
//!   let body: String = agent.get("http://example.com/page")
//!       .call()?
//!       .into_string()?;
//!
//!   // Reuses the connection from previous request.
//!   let response: String = agent.put("http://example.com/upload")
//!       .set("Authorization", "example-token")
//!       .call()?
//!       .into_string()?;
//! # Ok(())
//! # }
//! ```
//!
//! Ureq supports sending and receiving json, if you enable the "json" feature:
//!
//! ```rust
//! # #[cfg(feature = "json")]
//! # fn main() -> std::result::Result<(), ureq::Error> {
//! # ureq::is_test(true);
//!   // Requires the `json` feature enabled.
//!   let resp: String = ureq::post("http://myapi.example.com/ingest")
//!       .set("X-My-Header", "Secret")
//!       .send_json(ureq::json!({
//!           "name": "martin",
//!           "rust": true
//!       }))?
//!       .into_string()?;
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "json"))]
//! # fn main() {}
//! ```
//!
//! ## Error handling
//!
//! ureq returns errors via `Result<T, ureq::Error>`. That includes I/O errors,
//! protocol errors, and status code errors (when the server responded 4xx or
//! 5xx)
//!
//! ```rust
//! use ureq::Error;
//!
//! # fn req() {
//! match ureq::get("http://mypage.example.com/").call() {
//!     Ok(response) => { /* it worked */},
//!     Err(Error::Status(code, response)) => {
//!         /* the server returned an unexpected status
//!            code (such as 400, 500 etc) */
//!     }
//!     Err(_) => { /* some kind of io/transport error */ }
//! }
//! # }
//! # fn main() {}
//! ```
//!
//! More details on the [Error] type.
//!
//! ## Features
//!
//! To enable a minimal dependency tree, some features are off by default.
//! You can control them when including ureq as a dependency.
//!
//! `ureq = { version = "*", features = ["json", "charset"] }`
//!
//! * `tls` enables https. This is enabled by default.
//! * `cookies` enables cookies.
//! * `json` enables [Response::into_json()] and [Request::send_json()] via serde_json.
//! * `charset` enables interpreting the charset part of the Content-Type header
//!    (e.g.  `Content-Type: text/plain; charset=iso-8859-1`). Without this, the
//!    library defaults to Rust's built in `utf-8`.
//!
//! # Plain requests
//!
//! Most standard methods (GET, POST, PUT etc), are supported as functions from the
//! top of the library ([get()], [post()], [put()], etc).
//!
//! These top level http method functions create a [Request] instance
//! which follows a build pattern. The builders are finished using:
//!
//! * [`.call()`][Request::call()] without a request body.
//! * [`.send()`][Request::send()] with a request body as [Read][std::io::Read] (chunked encoding support for non-known sized readers).
//! * [`.send_string()`][Request::send_string()] body as string.
//! * [`.send_bytes()`][Request::send_bytes()] body as bytes.
//! * [`.send_form()`][Request::send_form()] key-value pairs as application/x-www-form-urlencoded.
//!
//! # JSON
//!
//! By enabling the `ureq = { version = "*", features = ["json"] }` feature,
//! the library supports serde json.
//!
//! * [`request.send_json()`][Request::send_json()] send body as serde json.
//! * [`response.into_json()`][Response::into_json()] transform response to json.
//!
//! # Content-Length and Transfer-Encoding
//!
//! The library will send a Content-Length header on requests with bodies of
//! known size, in other words, those sent with
//! [`.send_string()`][Request::send_string()],
//! [`.send_bytes()`][Request::send_bytes()],
//! [`.send_form()`][Request::send_form()], or
//! [`.send_json()`][Request::send_json()]. If you send a
//! request body with [`.send()`][Request::send()],
//! which takes a [Read][std::io::Read] of unknown size, ureq will send Transfer-Encoding:
//! chunked, and encode the body accordingly. Bodyless requests
//! (GETs and HEADs) are sent with [`.call()`][Request::call()]
//! and ureq adds neither a Content-Length nor a Transfer-Encoding header.
//!
//! If you set your own Content-Length or Transfer-Encoding header before
//! sending the body, ureq will respect that header by not overriding it,
//! and by encoding the body or not, as indicated by the headers you set.
//!
//! ```
//! let resp = ureq::post("http://my-server.com/ingest")
//!     .set("Transfer-Encoding", "chunked")
//!     .send_string("Hello world");
//! ```
//!
//! # Character encoding
//!
//! By enabling the `ureq = { version = "*", features = ["charset"] }` feature,
//! the library supports sending/receiving other character sets than `utf-8`.
//!
//! For [`response.into_string()`][Response::into_string()] we read the
//! header `Content-Type: text/plain; charset=iso-8859-1` and if it contains a charset
//! specification, we try to decode the body using that encoding. In the absence of, or failing
//! to interpret the charset, we fall back on `utf-8`.
//!
//! Similarly when using [`request.send_string()`][Request::send_string()],
//! we first check if the user has set a `; charset=<whatwg charset>` and attempt
//! to encode the request body using that.
//!
//! # Blocking I/O for simplicity
//!
//! Ureq uses blocking I/O rather than Rust's newer [asynchronous (async) I/O][async]. Async I/O
//! allows serving many concurrent requests without high costs in memory and OS threads. But
//! it comes at a cost in complexity. Async programs need to pull in a runtime (usually
//! [async-std] or [tokio]). They also need async variants of any method that might block, and of
//! [any method that might call another method that might block][what-color]. That means async
//! programs usually have a lot of dependencies - which adds to compile times, and increases
//! risk.
//!
//! The costs of async are worth paying, if you're writing an HTTP server that must serve
//! many many clients with minimal overhead. However, for HTTP _clients_, we believe that the
//! cost is usually not worth paying. The low-cost alternative to async I/O is blocking I/O,
//! which has a different price: it requires an OS thread per concurrent request. However,
//! that price is usually not high: most HTTP clients make requests sequentially, or with
//! low concurrency.
//!
//! That's why ureq uses blocking I/O and plans to stay that way. Other HTTP clients offer both
//! an async API and a blocking API, but we want to offer a blocking API without pulling in all
//! the dependencies required by an async API.
//!
//! [async]: https://rust-lang.github.io/async-book/01_getting_started/02_why_async.html
//! [async-std]: https://github.com/async-rs/async-std#async-std
//! [tokio]: https://github.com/tokio-rs/tokio#tokio
//! [what-color]: https://journal.stuffwithstuff.com/2015/02/01/what-color-is-your-function/
//!
//! ------------------------------------------------------------------------------
//!
//! Ureq is inspired by other great HTTP clients like
//! [superagent](http://visionmedia.github.io/superagent/) and
//! [the fetch API](https://developer.mozilla.org/en-US/docs/Web/API/Fetch_API).
//!
//! If ureq is not what you're looking for, check out these other Rust HTTP clients:
//! [surf](https://crates.io/crates/surf), [reqwest](https://crates.io/crates/reqwest),
//! [isahc](https://crates.io/crates/isahc), [attohttpc](https://crates.io/crates/attohttpc),
//! [actix-web](https://crates.io/crates/actix-web), and [hyper](https://crates.io/crates/hyper).
//!

mod agent;
mod body;
mod error;
mod header;
mod pool;
mod proxy;
mod request;
mod resolve;
mod response;
mod stream;
mod unit;

#[cfg(feature = "cookies")]
mod cookies;

#[cfg(feature = "json")]
pub use serde_json::json;
use url::Url;

#[cfg(test)]
mod test;
#[doc(hidden)]
mod testserver;

pub use crate::agent::Agent;
pub use crate::agent::AgentBuilder;
pub use crate::error::{Error, ErrorKind, Transport};
pub use crate::header::Header;
pub use crate::proxy::Proxy;
pub use crate::request::Request;
pub use crate::resolve::Resolver;
pub use crate::response::Response;

// re-export
#[cfg(feature = "cookies")]
pub use cookie::Cookie;
#[cfg(feature = "json")]
pub use serde_json::{to_value as serde_to_value, Map as SerdeMap, Value as SerdeValue};

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};

/// Creates an [AgentBuilder].
pub fn builder() -> AgentBuilder {
    AgentBuilder::new()
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
    let x = IS_TEST.load(Ordering::SeqCst);
    x
}

/// Agents are used to hold configuration and keep state between requests.
pub fn agent() -> Agent {
    #[cfg(not(test))]
    if is_test(false) {
        testserver::test_agent()
    } else {
        AgentBuilder::new().build()
    }
    #[cfg(test)]
    testserver::test_agent()
}

/// Make a request with the HTTP verb as a parameter.
///
/// This allows making requests with verbs that don't have a dedicated
/// method.
///
/// If you've got an already-parsed [Url], try [request_url][request_url].
///
/// ```
/// # fn main() -> Result<(), ureq::Error> {
/// # ureq::is_test(true);
/// let resp: ureq::Response = ureq::request("OPTIONS", "http://example.com/")
///     .call()?;
/// # Ok(())
/// # }
/// ```
pub fn request(method: &str, path: &str) -> Request {
    agent().request(method, path)
}
/// Make a request using an already-parsed [Url].
///
/// This is useful if you've got a parsed Url from some other source, or if
/// you want to parse the URL and then modify it before making the request.
/// If you'd just like to pass a String or a `&str`, try [request][request()].
///
/// ```
/// # fn main() -> Result<(), ureq::Error> {
/// # ureq::is_test(true);
/// use url::Url;
/// let agent = ureq::agent();
///
/// let mut url: Url = "http://example.com/some-page".parse().unwrap();
/// url.set_path("/robots.txt");
/// let resp: ureq::Response = ureq::request_url("GET", &url)
///     .call()?;
/// # Ok(())
/// # }
/// ```
pub fn request_url(method: &str, url: &Url) -> Request {
    agent().request_url(method, url)
}

/// Make a GET request.
pub fn get(path: &str) -> Request {
    request("GET", path)
}

/// Make a HEAD request.
pub fn head(path: &str) -> Request {
    request("HEAD", path)
}

/// Make a POST request.
pub fn post(path: &str) -> Request {
    request("POST", path)
}

/// Make a PUT request.
pub fn put(path: &str) -> Request {
    request("PUT", path)
}

/// Make a DELETE request.
pub fn delete(path: &str) -> Request {
    request("DELETE", path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_http_google() {
        let agent = Agent::new();

        let resp = agent.get("http://www.google.com/").call().unwrap();
        assert_eq!(
            "text/html; charset=ISO-8859-1",
            resp.header("content-type").unwrap()
        );
        assert_eq!("text/html", resp.content_type());
    }

    #[test]
    #[cfg(feature = "tls")]
    fn connect_https_google() {
        let agent = Agent::new();

        let resp = agent.get("https://www.google.com/").call().unwrap();
        assert_eq!(
            "text/html; charset=ISO-8859-1",
            resp.header("content-type").unwrap()
        );
        assert_eq!("text/html", resp.content_type());
    }

    #[test]
    #[cfg(feature = "tls")]
    fn connect_https_invalid_name() {
        let result = get("https://example.com{REQUEST_URI}/").call();
        let e = ErrorKind::Dns;
        assert_eq!(result.unwrap_err().kind(), e);
    }
}
