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
//! HTTPS, charset decoding, and is based on the API of the `http` crate.
//!
//! Ureq is in pure Rust for safety and ease of understanding. It avoids using
//! `unsafe` directly. It uses blocking I/O instead of async I/O, because that keeps
//! the API simple and keeps dependencies to a minimum. For TLS, ureq uses
//! rustls or native-tls.
//!
//! See the [changelog] for details of recent releases.
//!
//! [changelog]: https://github.com/algesten/ureq/blob/main/CHANGELOG.md
//!
//! # Usage
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
//! For more involved tasks, you'll want to create an [`Agent`]. An Agent
//! holds a connection pool for reuse, and a cookie store if you use the
//! **cookies** feature. An Agent can be cheaply cloned due to internal
//! [`Arc`] and all clones of an Agent share state among each other. Creating
//! an Agent also allows setting options like the TLS configuration.
//!
//! ```rust
//! # fn no_run() -> Result<(), ureq::Error> {
//! use ureq::Agent;
//! use std::time::Duration;
//!
//! let mut config = Agent::config_builder()
//!     .timeout_global(Some(Duration::from_secs(5)))
//!     .build();
//!
//! let agent: Agent = config.into();
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
//! # Ok(())}
//! ```
//!
//! ## JSON
//!
//! Ureq supports sending and receiving json, if you enable the **json** feature:
//!
//! ```rust
//! # #[cfg(feature = "json")]
//! # fn no_run() -> Result<(), ureq::Error> {
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
//! # Ok(())}
//! ```
//!
//! ## Error handling
//!
//! ureq returns errors via `Result<T, ureq::Error>`. That includes I/O errors,
//! protocol errors. By default, also HTTP status code errors (when the
//! server responded 4xx or 5xx) results in [`Error`].
//!
//! This behavior can be turned off via [`http_status_as_error()`]
//!
//! ```rust
//! use ureq::Error;
//!
//! # fn no_run() -> Result<(), ureq::Error> {
//! match ureq::get("http://mypage.example.com/").call() {
//!     Ok(response) => { /* it worked */},
//!     Err(Error::StatusCode(code)) => {
//!         /* the server returned an unexpected status
//!            code (such as 400, 500 etc) */
//!     }
//!     Err(_) => { /* some kind of io/transport/etc error */ }
//! }
//! # Ok(())}
//! ```
//!
//! # Features
//!
//! To enable a minimal dependency tree, some features are off by default.
//! You can control them when including ureq as a dependency.
//!
//! `ureq = { version = "3", features = ["socks-proxy", "charset"] }`
//!
//! The default enabled features are: **rustls** and **gzip**.
//!
//! * **rustls** enables the rustls TLS implementation. This is the default for the the crate level
//!   convenience calls (`ureq::get` etc). It currently uses `ring` as the TLS provider.
//! * **native-tls** enables the native tls backend for TLS. Due to the risk of diamond dependencies
//!   accidentally switching on an unwanted TLS implementation, `native-tls` is never picked up as
//!   a default or used by the crate level convenience calls (`ureq::get` etc) – it must be configured
//!   on the agent
//! * **platform-verifier** enables verifying the server certificates using a method native to the
//!   platform ureq is executing on. See [rustls-platform-verifier] crate
//! * **socks-proxy** enables proxy config using the `socks4://`, `socks4a://`, `socks5://`
//!   and `socks://` (equal to `socks5://`) prefix
//! * **cookies** enables cookies
//! * **gzip** enables requests of gzip-compressed responses and decompresses them
//! * **brotli** enables requests brotli-compressed responses and decompresses them
//! * **charset** enables interpreting the charset part of the Content-Type header
//!   (e.g.  `Content-Type: text/plain; charset=iso-8859-1`). Without this, the
//!   library defaults to Rust's built in `utf-8`
//! * **json** enables JSON sending and receiving via serde_json
//! * **multipart** enables multipart/form-data sending via [`unversioned::multipart`]
//! * **rustls-webpki-roots** enables the webpki-roots crate for root certificates when using rustls.
//! * **native-tls-webpki-roots** enables the webpki-root-certs crate for root certificates when using native-tls.
//!
//! ### Unstable
//!
//! These features are unstable and might change in a minor version.
//!
//! * **rustls-no-provider** Enables rustls, but does not enable webpki and any [`CryptoProvider`] such as `ring`.
//!   Root certs and providers other than the default (currently `ring`) are never picked up from feature flags alone.
//!   They must be configured on the agent.
//!
//! * **native-tls-no-default** Enables native-tls, but does not enable webpki.
//!   Root certs are never picked up from feature flags alone. They must be configured on the agent.
//!
//! * **vendored** compiles and statically links to a copy of non-Rust vendors (e.g. OpenSSL from `native-tls`)
//!
//! # TLS (https)
//!
//! ## rustls
//!
//! By default, ureq uses [`rustls` crate] with the `ring` cryptographic provider.
//! As of Sep 2024, the `ring` provider has a higher chance of compiling successfully. If the user
//! installs another process [default provider], that choice is respected.
//!
//! ureq does not guarantee to default to ring indefinitely. `rustls` as a feature flag will always
//! work, but the specific crypto backend might change in a minor version.
//!
//! ```
//! # #[cfg(feature = "rustls")]
//! # {
//! // This uses rustls
//! ureq::get("https://www.google.com/").call().unwrap();
//! # } Ok::<_, ureq::Error>(())
//! ```
//!
//! ### rustls without ring
//!
//! ureq never changes TLS backend from feature flags alone. It is possible to compile ureq
//! without ring, but it requires specific feature flags and configuring the [`Agent`].
//!
//! Since rustls is not semver 1.x, this requires non-semver-guaranteed API. I.e. ureq might
//! change this behavior without a major version bump.
//!
//! Read more at [`TlsConfigBuilder::unversioned_rustls_crypto_provider`][crate::tls::TlsConfigBuilder::unversioned_rustls_crypto_provider].
//!
//! ## native-tls
//!
//! As an alternative, ureq ships with [`native-tls`] as a TLS provider. This must be
//! enabled using the **native-tls** feature. Due to the risk of diamond dependencies
//! accidentally switching on an unwanted TLS implementation, `native-tls` is never picked
//! up as a default or used by the crate level convenience calls (`ureq::get` etc) – it
//! must be configured on the agent.
//!
//! ```
//! # #[cfg(feature = "native-tls")]
//! # {
//! use ureq::config::Config;
//! use ureq::tls::{TlsConfig, TlsProvider};
//!
//! let mut config = Config::builder()
//!     .tls_config(
//!         TlsConfig::builder()
//!             // requires the native-tls feature
//!             .provider(TlsProvider::NativeTls)
//!             .build()
//!     )
//!     .build();
//!
//! let agent = config.new_agent();
//!
//! agent.get("https://www.google.com/").call().unwrap();
//! # } Ok::<_, ureq::Error>(())
//! ```
//!
//! ## Root certificates
//!
//! ### webpki-roots
//!
//! By default, ureq uses Mozilla's root certificates via the [webpki-roots] crate. This is a static
//! bundle of root certificates that do not update automatically. It also circumvents whatever root
//! certificates are installed on the host running ureq, which might be a good or a bad thing depending
//! on your perspective. There is also no mechanism for [SCT], [CRL]s or other revocations.
//! To maintain a "fresh" list of root certs, you need to bump the ureq dependency from time to time.
//!
//! The main reason for chosing this as the default is to minimize the number of dependencies. More
//! details about this decision can be found at [PR 818].
//!
//! If your use case for ureq is talking to a limited number of servers with high trust, the
//! default setting is likely sufficient. If you use ureq with a high number of servers, or servers
//! you don't trust, we recommend using the platform verifier (see below).
//!
//! ### platform-verifier
//!
//! The [rustls-platform-verifier] crate provides access to natively checking the certificate via your OS.
//! To use this verifier, you need to enable it using feature flag **platform-verifier** as well as
//! configure an agent to use it.
//!
//! ```
//! # #[cfg(all(feature = "rustls", feature="platform-verifier"))]
//! # {
//! use ureq::Agent;
//! use ureq::tls::{TlsConfig, RootCerts};
//!
//! let agent = Agent::config_builder()
//!     .tls_config(
//!         TlsConfig::builder()
//!             .root_certs(RootCerts::PlatformVerifier)
//!             .build()
//!     )
//!     .build()
//!     .new_agent();
//!
//! let response = agent.get("https://httpbin.org/get").call()?;
//! # } Ok::<_, ureq::Error>(())
//! ```
//!
//! Setting `RootCerts::PlatformVerifier` together with `TlsProvider::NativeTls` means
//! also native-tls will use the OS roots instead of [webpki-roots] crate. Whether that
//! results in a config that has CRLs and revocations is up to whatever native-tls links to.
//!
//! # JSON
//!
//! By enabling the **json** feature, the library supports serde json.
//!
//! This is enabled by default.
//!
//! * [`request.send_json()`] send body as json.
//! * [`body.read_json()`] transform response to json.
//!
//! # Sending body data
//!
//! HTTP/1.1 has two ways of transfering body data. Either of a known size with
//! the `Content-Length` HTTP header, or unknown size with the
//! `Transfer-Encoding: chunked` header. ureq supports both and will use the
//! appropriate method depending on which body is being sent.
//!
//! ureq has a [`AsSendBody`] trait that is implemented for many well known types
//! of data that we might want to send. The request body can thus be anything
//! from a `String` to a `File`, see below.
//!
//! ## Content-Length
//!
//! The library will send a `Content-Length` header on requests with bodies of
//! known size, in other words, if the body to send is one of:
//!
//! * `&[u8]`
//! * `&[u8; N]`
//! * `&str`
//! * `String`
//! * `&String`
//! * `Vec<u8>`
//! * `&Vec<u8>)`
//! * [`SendBody::from_json()`] (implicitly via [`request.send_json()`])
//!
//! ## Transfer-Encoding: chunked
//!
//! ureq will send a `Transfer-Encoding: chunked` header on requests where the body
//! is of unknown size. The body is automatically converted to an [`std::io::Read`]
//! when the type is one of:
//!
//! * `File`
//! * `&File`
//! * `TcpStream`
//! * `&TcpStream`
//! * `Stdin`
//! * `UnixStream` (not on windows)
//!
//! ### From readers
//!
//! The chunked method also applies for bodies constructed via:
//!
//! * [`SendBody::from_reader()`]
//! * [`SendBody::from_owned_reader()`]
//!
//! ## Proxying a response body
//!
//! As a special case, when ureq sends a [`Body`] from a previous http call, the
//! use of `Content-Length` or `chunked` depends on situation. For input such as
//! gzip decoding (**gzip** feature) or charset transformation (**charset** feature),
//! the output body might not match the input, which means ureq is forced to use
//! the `chunked` method.
//!
//! * `Response<Body>`
//!
//! ## Sending form data
//!
//! [`request.send_form()`] provides a way to send `application/x-www-form-urlencoded`
//! encoded data. The key/values provided will be URL encoded.
//!
//! ## Overriding
//!
//! If you set your own Content-Length or Transfer-Encoding header before
//! sending the body, ureq will respect that header by not overriding it,
//! and by encoding the body or not, as indicated by the headers you set.
//!
//! ```
//! let resp = ureq::put("https://httpbin.org/put")
//!     .header("Transfer-Encoding", "chunked")
//!     .send("Hello world")?;
//! # Ok::<_, ureq::Error>(())
//! ```
//!
//! # Character encoding
//!
//! By enabling the **charset** feature, the library supports receiving other
//! character sets than `utf-8`.
//!
//! For [`Body::read_to_string()`] we read the header like:
//!
//! `Content-Type: text/plain; charset=iso-8859-1`
//!
//! and if it contains a charset specification, we try to decode the body using that
//! encoding. In the absence of, or failing to interpret the charset, we fall back on `utf-8`.
//!
//! Currently ureq does not provide a way to encode when sending request bodies.
//!
//! ## Lossy utf-8
//!
//! When reading text bodies (with a `Content-Type` starting `text/` as in `text/plain`,
//! `text/html`, etc), ureq can ensure the body is possible to read as a `String` also if
//! it contains characters that are not valid for utf-8. Invalid characters are replaced
//! with a question mark `?` (NOT the utf-8 replacement character).
//!
//! For [`Body::read_to_string()`] this is turned on by default, but it can be disabled
//! and conversely for [`Body::as_reader()`] it is not enabled, but can be.
//!
//! To precisely configure the behavior use [`Body::with_config()`].
//!
//! # Proxying
//!
//! ureq supports two kinds of proxies,  [`HTTP`] ([`CONNECT`]), [`SOCKS4`]/[`SOCKS5`],
//! the former is always available while the latter must be enabled using the feature
//! **socks-proxy**.
//!
//! Proxies settings are configured on an [`Agent`]. All request sent through the agent will be proxied.
//!
//! ## Environment Variables
//!
//! ureq automatically reads proxy configuration from environment variables when creating
//! a default [`Agent`]. Proxy variables are checked in order: `ALL_PROXY`, `HTTPS_PROXY`,
//! then `HTTP_PROXY` (with lowercase variants).
//!
//! `NO_PROXY` specifies hosts that bypass the proxy, supporting exact hosts, wildcard
//! suffixes (`*.example.com`), dot suffixes (`.example.com`), and match-all (`*`).
//!
//! ## Example using HTTP
//!
//! ```rust
//! use ureq::{Agent, Proxy};
//! # fn no_run() -> std::result::Result<(), ureq::Error> {
//! // Configure an http connect proxy.
//! let proxy = Proxy::new("http://user:password@cool.proxy:9090")?;
//! let agent: Agent = Agent::config_builder()
//!     .proxy(Some(proxy))
//!     .build()
//!     .into();
//!
//! // This is proxied.
//! let resp = agent.get("http://cool.server").call()?;
//! # Ok(())}
//! # fn main() {}
//! ```
//!
//! ## Example using SOCKS5
//!
//! ```rust
//! use ureq::{Agent, Proxy};
//! # #[cfg(feature = "socks-proxy")]
//! # fn no_run() -> std::result::Result<(), ureq::Error> {
//! // Configure a SOCKS proxy.
//! let proxy = Proxy::new("socks5://user:password@cool.proxy:9090")?;
//! let agent: Agent = Agent::config_builder()
//!     .proxy(Some(proxy))
//!     .build()
//!     .into();
//!
//! // This is proxied.
//! let resp = agent.get("http://cool.server").call()?;
//! # Ok(())}
//! ```
//!
//! # Log levels
//!
//! ureq uses the log crate. These are the definitions of the log levels, however we
//! do not guarantee anything for dependencies such as `http` and `rustls`.
//!
//! * `ERROR` - nothing
//! * `WARN` - if we detect a user configuration problem.
//! * `INFO` - nothing
//! * `DEBUG` - uri, state changes, transport, resolver and selected request/response headers
//! * `TRACE` - wire level debug. NOT REDACTED!
//!
//! The request/response headers on DEBUG levels are allow-listed to only include headers that
//! are considered safe. The code has the [allow list](https://github.com/algesten/ureq/blob/81127cfc38516903330dc1b9c618122372f8dc29/src/util.rs#L184-L198).
//!
//! # Versioning
//!
//! ## Semver and `unversioned`
//!
//! ureq follows semver. From ureq 3.x we strive to have a much closer adherence to semver than 2.x.
//! The main mistake in 2.x was to re-export crates that were not yet semver 1.0. In ureq 3.x TLS and
//! cookie configuration is shimmed using our own types.
//!
//! ureq 3.x is trying out two new traits that had no equivalent in 2.x, [`Transport`] and [`Resolver`].
//! These allow the user write their own bespoke transports and (DNS name) resolver. The API:s for
//! these parts are not yet solidified. They live under the [`unversioned`] module, and do not
//! follow semver. See module doc for more info.
//!
//! ## Breaking changes in dependencies
//!
//! ureq relies on non-semver 1.x crates such as `rustls` and `native-tls`. Some scenarios, such
//! as configuring `rustls` to not use `ring`, a user of ureq might need to interact with these
//! crates directly instead of going via ureq's provided API.
//!
//! Such changes can break when ureq updates dependencies. This is not considered a breaking change
//! for ureq and will not be reflected by a major version bump.
//!
//! We strive to mark ureq's API with the word "unversioned" to identify places where this risk arises.
//!
//! ## Minimum Supported Rust Version (MSRV)
//!
//! From time to time we will need to update our minimum supported Rust version (MSRV). This is not
//! something we do lightly; our ambition is to be as conservative with MSRV as possible.
//!
//! * For some dependencies, we will opt for pinning the version of the dep instead
//!   of bumping our MSRV.
//! * For important dependencies, like the TLS libraries, we cannot hold back our MSRV if they change.
//! * We do not consider MSRV changes to be breaking for the purposes of semver.
//! * We will not make MSRV changes in patch releases.
//! * MSRV changes will get their own minor release, and not be co-mingled with other changes.
//!
//! [`HTTP`]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Proxy_servers_and_tunneling#http_tunneling
//! [`CONNECT`]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Methods/CONNECT
//! [`SOCKS4`]: https://en.wikipedia.org/wiki/SOCKS#SOCKS4
//! [`SOCKS5`]: https://en.wikipedia.org/wiki/SOCKS#SOCKS5
//! [`rustls` crate]: https://crates.io/crates/rustls
//! [default provider]: https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html#method.install_default
//! [`native-tls`]: https://crates.io/crates/native-tls
//! [rustls-platform-verifier]: https://crates.io/crates/rustls-platform-verifier
//! [webpki-roots]: https://crates.io/crates/webpki-roots
//! [`Arc`]: https://doc.rust-lang.org/std/sync/struct.Arc.html
//! [`Agent`]: https://docs.rs/ureq/latest/ureq/struct.Agent.html
//! [`Error`]: https://docs.rs/ureq/latest/ureq/enum.Error.html
//! [`http_status_as_error()`]: https://docs.rs/ureq/latest/ureq/config/struct.ConfigBuilder.html#method.http_status_as_error
//! [SCT]: https://en.wikipedia.org/wiki/Certificate_Transparency
//! [CRL]: https://en.wikipedia.org/wiki/Certificate_revocation_list
//! [PR 818]: https://github.com/algesten/ureq/pull/818
//! [`request.send_json()`]: https://docs.rs/ureq/latest/ureq/struct.RequestBuilder.html#method.send_json
//! [`body.read_json()`]: https://docs.rs/ureq/latest/ureq/struct.Body.html#method.read_json
//! [`AsSendBody`]: https://docs.rs/ureq/latest/ureq/trait.AsSendBody.html
//! [`SendBody::from_json()`]: https://docs.rs/ureq/latest/ureq/struct.SendBody.html#method.from_json
//! [`std::io::Read`]: https://doc.rust-lang.org/std/io/trait.Read.html
//! [`SendBody::from_reader()`]: https://docs.rs/ureq/latest/ureq/struct.SendBody.html#method.from_reader
//! [`SendBody::from_owned_reader()`]: https://docs.rs/ureq/latest/ureq/struct.SendBody.html#method.from_owned_reader
//! [`Body`]: https://docs.rs/ureq/latest/ureq/struct.Body.html
//! [`request.send_form()`]: https://docs.rs/ureq/latest/ureq/struct.RequestBuilder.html#method.send_form
//! [`Body::read_to_string()`]: https://docs.rs/ureq/latest/ureq/struct.Body.html#method.read_to_string
//! [`Body::as_reader()`]: https://docs.rs/ureq/latest/ureq/struct.Body.html#method.as_reader
//! [`Body::with_config()`]: https://docs.rs/ureq/latest/ureq/struct.Body.html#method.with_config
//! [`Transport`]: https://docs.rs/ureq/latest/ureq/unversioned/transport/trait.Transport.html
//! [`Resolver`]: https://docs.rs/ureq/latest/ureq/unversioned/resolver/trait.Resolver.html
//! [`unversioned`]: https://docs.rs/ureq/latest/ureq/unversioned/index.html
//! [`CryptoProvider`]: https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html
//! [`unversioned::multipart`]: https://docs.rs/ureq/latest/ureq/unversioned/multipart/index.html

#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![deny(missing_docs)]
// I don't think elided lifetimes help in understanding the code.
#![allow(clippy::needless_lifetimes)]
// Since you can't use inlined args for all cases, using it means the
// code will have a mix of inlined and not inlined. Code should be
// uniform, thus this lint is misguided.
#![allow(clippy::uninlined_format_args)]
#![allow(mismatched_lifetime_syntaxes)]

#[macro_use]
extern crate log;

use std::convert::TryFrom;

/// Re-exported http-crate.
pub use ureq_proto::http;

pub use body::{Body, BodyBuilder, BodyReader, BodyWithConfig};
use http::Method;
use http::{Request, Response, Uri};
pub use proxy::{Proxy, ProxyBuilder, ProxyProtocol};
pub use request::RequestBuilder;
use request::{WithBody, WithoutBody};
pub use request_ext::RequestExt;
pub use response::ResponseExt;
pub use send_body::AsSendBody;

mod agent;
mod body;
pub mod config;
mod error;
mod pool;
mod proxy;
mod query;
mod request;
mod response;
mod run;
mod send_body;
mod timings;
mod util;

pub mod unversioned;
use unversioned::resolver;
use unversioned::transport;

pub mod middleware;

#[cfg(feature = "_tls")]
pub mod tls;

#[cfg(feature = "cookies")]
mod cookies;
mod request_ext;

#[cfg(feature = "cookies")]
pub use cookies::{Cookie, CookieJar};

pub use agent::Agent;
pub use error::Error;
pub use send_body::SendBody;
pub use timings::Timeout;

/// Typestate variables.
pub mod typestate {
    pub use super::request::WithBody;
    pub use super::request::WithoutBody;

    pub use super::config::typestate::AgentScope;
    pub use super::config::typestate::HttpCrateScope;
    pub use super::config::typestate::RequestScope;
}

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

/// Make a GET request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn get<T>(uri: T) -> RequestBuilder<WithoutBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithoutBody>::new(Agent::new_with_defaults(), Method::GET, uri)
}

/// Make a POST request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn post<T>(uri: T) -> RequestBuilder<WithBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithBody>::new(Agent::new_with_defaults(), Method::POST, uri)
}

/// Make a PUT request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn put<T>(uri: T) -> RequestBuilder<WithBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithBody>::new(Agent::new_with_defaults(), Method::PUT, uri)
}

/// Make a DELETE request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn delete<T>(uri: T) -> RequestBuilder<WithoutBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithoutBody>::new(Agent::new_with_defaults(), Method::DELETE, uri)
}

/// Make a HEAD request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn head<T>(uri: T) -> RequestBuilder<WithoutBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithoutBody>::new(Agent::new_with_defaults(), Method::HEAD, uri)
}

/// Make an OPTIONS request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn options<T>(uri: T) -> RequestBuilder<WithoutBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithoutBody>::new(Agent::new_with_defaults(), Method::OPTIONS, uri)
}

/// Make a CONNECT request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn connect<T>(uri: T) -> RequestBuilder<WithoutBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithoutBody>::new(Agent::new_with_defaults(), Method::CONNECT, uri)
}

/// Make a PATCH request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn patch<T>(uri: T) -> RequestBuilder<WithBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithBody>::new(Agent::new_with_defaults(), Method::PATCH, uri)
}

/// Make a TRACE request.
///
/// Run on a use-once [`Agent`].
#[must_use]
pub fn trace<T>(uri: T) -> RequestBuilder<WithoutBody>
where
    Uri: TryFrom<T>,
    <Uri as TryFrom<T>>::Error: Into<http::Error>,
{
    RequestBuilder::<WithoutBody>::new(Agent::new_with_defaults(), Method::TRACE, uri)
}

#[cfg(test)]
pub(crate) mod test {
    use std::{io, sync::OnceLock};

    use assert_no_alloc::AllocDisabler;
    use config::{Config, ConfigBuilder};
    use typestate::AgentScope;

    use super::*;

    #[global_allocator]
    // Some tests checks that we are not allocating
    static A: AllocDisabler = AllocDisabler;

    pub fn init_test_log() {
        static INIT_LOG: OnceLock<()> = OnceLock::new();
        INIT_LOG.get_or_init(env_logger::init);
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
        use config::Config;

        use crate::tls::{TlsConfig, TlsProvider};

        let agent: Agent = Config::builder()
            .tls_config(TlsConfig::builder().provider(TlsProvider::Rustls).build())
            .build()
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
    fn connect_https_google_native_tls_simple() {
        init_test_log();
        use config::Config;

        use crate::tls::{TlsConfig, TlsProvider};

        let agent: Agent = Config::builder()
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    .build(),
            )
            .build()
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
    #[cfg(feature = "rustls")]
    fn connect_https_google_rustls_webpki() {
        init_test_log();
        use crate::tls::{RootCerts, TlsConfig, TlsProvider};
        use config::Config;

        let agent: Agent = Config::builder()
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::Rustls)
                    .root_certs(RootCerts::WebPki)
                    .build(),
            )
            .build()
            .into();

        agent.get("https://www.google.com/").call().unwrap();
    }

    #[test]
    #[cfg(feature = "native-tls")]
    fn connect_https_google_native_tls_webpki() {
        init_test_log();
        use crate::tls::{RootCerts, TlsConfig, TlsProvider};
        use config::Config;

        let agent: Agent = Config::builder()
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    .root_certs(RootCerts::WebPki)
                    .build(),
            )
            .build()
            .into();

        agent.get("https://www.google.com/").call().unwrap();
    }

    #[test]
    #[cfg(feature = "rustls")]
    fn connect_https_google_noverif() {
        init_test_log();
        use crate::tls::{TlsConfig, TlsProvider};

        let agent: Agent = Config::builder()
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::Rustls)
                    .disable_verification(true)
                    .build(),
            )
            .build()
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
    fn simple_get() {
        init_test_log();
        let mut res = get("http://httpbin.org/get").call().unwrap();
        res.body_mut().read_to_string().unwrap();
    }

    #[test]
    fn query_no_slash() {
        init_test_log();
        let mut res = get("http://httpbin.org?query=foo").call().unwrap();
        res.body_mut().read_to_string().unwrap();
    }

    #[test]
    fn simple_head() {
        init_test_log();
        let mut res = head("http://httpbin.org/get").call().unwrap();
        res.body_mut().read_to_string().unwrap();
    }

    #[test]
    fn redirect_no_follow() {
        init_test_log();
        let agent: Agent = Config::builder().max_redirects(0).build().into();
        let mut res = agent
            .get("http://httpbin.org/redirect-to?url=%2Fget")
            .call()
            .unwrap();
        let txt = res.body_mut().read_to_string().unwrap();
        #[cfg(feature = "_test")]
        assert_eq!(txt, "You've been redirected");
        #[cfg(not(feature = "_test"))]
        assert_eq!(txt, "");
    }

    #[test]
    fn test_send_form_content_type() {
        init_test_log();

        // These tests verify that the methods work correctly with the test infrastructure
        // The actual Content-Type verification happens at the transport level
        let form_data = [("key1", "value1"), ("key2", "value2")];
        let mut res = post("http://httpbin.org/post")
            .header("x-verify-content-type", "application/x-www-form-urlencoded")
            .send_form(form_data)
            .unwrap();

        let _txt = res.body_mut().read_to_string().unwrap();
    }

    #[test]
    #[cfg(feature = "json")]
    fn test_send_json_content_type() {
        use serde_json::json;

        init_test_log();

        let data = json!({"key": "value"});
        let mut res = post("http://httpbin.org/post")
            .header("x-verify-content-type", "application/json; charset=utf-8")
            .send_json(&data)
            .unwrap();

        let _txt = res.body_mut().read_to_string().unwrap();
    }

    #[test]
    #[cfg(feature = "multipart")]
    fn test_send_multipart_content_type() {
        use crate::unversioned::multipart::Form;

        init_test_log();

        let form = Form::new()
            .text("field1", "value1")
            .text("field2", "value2");

        let mut res = post("http://httpbin.org/post")
            .header("x-verify-content-type", "multipart/form-data")
            .send(form)
            .unwrap();

        let _txt = res.body_mut().read_to_string().unwrap();
    }

    #[test]
    fn redirect_max_with_error() {
        init_test_log();
        let agent: Agent = Config::builder().max_redirects(3).build().into();
        let res = agent
            .get(
                "http://httpbin.org/redirect-to?url=%2Fredirect-to%3F\
                url%3D%2Fredirect-to%3Furl%3D%252Fredirect-to%253Furl%253D",
            )
            .call();
        let err = res.unwrap_err();
        assert_eq!(err.to_string(), "too many redirects");
    }

    #[test]
    fn redirect_max_without_error() {
        init_test_log();
        let agent: Agent = Config::builder()
            .max_redirects(3)
            .max_redirects_will_error(false)
            .build()
            .into();
        let res = agent
            .get(
                "http://httpbin.org/redirect-to?url=%2Fredirect-to%3F\
                url%3D%2Fredirect-to%3Furl%3D%252Fredirect-to%253Furl%253D",
            )
            .call()
            .unwrap();
        assert_eq!(res.status(), 302);
    }

    #[test]
    fn redirect_follow() {
        init_test_log();
        let res = get("http://httpbin.org/redirect-to?url=%2Fget")
            .call()
            .unwrap();
        let response_uri = res.get_uri();
        assert_eq!(response_uri.path(), "/get")
    }

    #[test]
    #[cfg(feature = "_test")]
    fn post_redirect_to_get_removes_content_length() {
        // Regression test for issue #1135
        // When a POST with body redirects to GET, Content-Length should not be sent
        use crate::transport::{set_handler, set_handler_cb};
        init_test_log();

        // POST endpoint that redirects to GET
        set_handler(
            "/post-redirect",
            301,
            &[("Location", "http://example.com/get")],
            &[],
        );

        // GET endpoint - verifies Content-Length header is NOT present
        set_handler_cb("/get", 200, &[], b"success", |req| {
            // Assert that Content-Length is not present on the GET request
            assert!(
                req.headers().get("content-length").is_none(),
                "Content-Length header should not be present on GET request after redirect"
            );
        });

        // POST with body that redirects to GET
        let mut res = post("http://example.org/post-redirect")
            .send("test body")
            .unwrap();

        assert_eq!(res.status(), 200);
        let body = res.body_mut().read_to_string().unwrap();
        assert_eq!(body, "success");
    }

    #[test]
    fn redirect_history_none() {
        init_test_log();
        let res = get("http://httpbin.org/redirect-to?url=%2Fget")
            .call()
            .unwrap();
        let redirect_history = res.get_redirect_history();
        assert_eq!(redirect_history, None)
    }

    #[test]
    fn redirect_history_some() {
        init_test_log();
        let agent: Agent = Config::builder()
            .max_redirects(3)
            .max_redirects_will_error(false)
            .save_redirect_history(true)
            .build()
            .into();
        let res = agent
            .get("http://httpbin.org/redirect-to?url=%2Fget")
            .call()
            .unwrap();
        let redirect_history = res.get_redirect_history();
        assert_eq!(
            redirect_history,
            Some(
                vec![
                    "http://httpbin.org/redirect-to?url=%2Fget".parse().unwrap(),
                    "http://httpbin.org/get".parse().unwrap()
                ]
                .as_ref()
            )
        );
        let res = agent
            .get(
                "http://httpbin.org/redirect-to?url=%2Fredirect-to%3F\
                url%3D%2Fredirect-to%3Furl%3D%252Fredirect-to%253Furl%253D",
            )
            .call()
            .unwrap();
        let redirect_history = res.get_redirect_history();
        assert_eq!(
            redirect_history,
            Some(vec![
                "http://httpbin.org/redirect-to?url=%2Fredirect-to%3Furl%3D%2Fredirect-to%3Furl%3D%252Fredirect-to%253Furl%253D".parse().unwrap(),
                "http://httpbin.org/redirect-to?url=/redirect-to?url=%2Fredirect-to%3Furl%3D".parse().unwrap(),
                "http://httpbin.org/redirect-to?url=/redirect-to?url=".parse().unwrap(),
                "http://httpbin.org/redirect-to?url=".parse().unwrap(),
            ].as_ref())
        );
        let res = agent.get("https://www.google.com/").call().unwrap();
        let redirect_history = res.get_redirect_history();
        assert_eq!(
            redirect_history,
            Some(vec!["https://www.google.com/".parse().unwrap()].as_ref())
        );
    }

    #[test]
    fn connect_https_invalid_name() {
        let result = get("https://example.com{REQUEST_URI}/").call();
        let err = result.unwrap_err();
        assert!(matches!(err, Error::Http(_)));
        assert_eq!(err.to_string(), "http: invalid uri character");
    }

    #[test]
    fn post_big_body_chunked() {
        init_test_log();
        // https://github.com/algesten/ureq/issues/879
        let mut data = io::Cursor::new(vec![42; 153_600]);
        post("http://httpbin.org/post")
            .content_type("application/octet-stream")
            .send(SendBody::from_reader(&mut data))
            .expect("to send correctly");
    }

    #[test]
    #[cfg(not(feature = "_test"))]
    fn post_array_body_sends_content_length() {
        init_test_log();
        let mut response = post("http://httpbin.org/post")
            .content_type("application/octet-stream")
            .send(vec![42; 123])
            .expect("to send correctly");

        let ret = response.body_mut().read_to_string().unwrap();
        assert!(ret.contains("\"Content-Length\": \"123\""));
    }

    #[test]
    #[cfg(not(feature = "_test"))]
    fn post_file_sends_file_length() {
        init_test_log();

        let bytes = include_bytes!("../LICENSE-MIT");
        let file = std::fs::File::open("LICENSE-MIT").unwrap();

        let mut response = post("http://httpbin.org/post")
            .content_type("application/octet-stream")
            .send(file)
            .expect("to send correctly");

        let ret = response.body_mut().read_to_string().unwrap();
        assert!(ret.contains(&format!("\"Content-Length\": \"{}\"", bytes.len())));
    }

    #[test]
    #[cfg(not(feature = "_test"))]
    fn username_password_from_uri() {
        init_test_log();
        let mut res = get("https://martin:secret@httpbin.org/get").call().unwrap();
        let body = res.body_mut().read_to_string().unwrap();
        assert!(body.contains("Basic bWFydGluOnNlY3JldA=="));
    }

    #[test]
    #[cfg(all(feature = "cookies", feature = "_test"))]
    fn store_response_cookies() {
        let agent = Agent::new_with_defaults();
        let _ = agent.get("https://www.google.com").call().unwrap();

        let mut all: Vec<_> = agent
            .cookie_jar_lock()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        all.sort();

        assert_eq!(all, ["AEC", "__Secure-ENID"])
    }

    #[test]
    #[cfg(all(feature = "cookies", feature = "_test"))]
    fn send_request_cookies() {
        init_test_log();

        let agent = Agent::new_with_defaults();
        let uri = Uri::from_static("http://cookie.test/cookie-test");
        let uri2 = Uri::from_static("http://cookie2.test/cookie-test");

        let mut jar = agent.cookie_jar_lock();
        jar.insert(Cookie::parse("a=1", &uri).unwrap(), &uri)
            .unwrap();
        jar.insert(Cookie::parse("b=2", &uri).unwrap(), &uri)
            .unwrap();
        jar.insert(Cookie::parse("c=3", &uri2).unwrap(), &uri2)
            .unwrap();

        jar.release();

        let _ = agent.get("http://cookie.test/cookie-test").call().unwrap();
    }

    #[test]
    #[cfg(all(feature = "_test", not(feature = "cookies")))]
    fn partial_redirect_when_following() {
        init_test_log();
        // this should work because we follow the redirect and go to /get
        get("http://my-host.com/partial-redirect").call().unwrap();
    }

    #[test]
    #[cfg(feature = "_test")]
    fn partial_redirect_when_not_following() {
        init_test_log();
        // this should fail because we are not following redirects, and the
        // response is partial before the server is hanging up
        get("http://my-host.com/partial-redirect")
            .config()
            .max_redirects(0)
            .build()
            .call()
            .unwrap_err();
    }

    #[test]
    #[cfg(feature = "_test")]
    fn http_connect_proxy() {
        init_test_log();

        let proxy = Proxy::new("http://my_proxy:1234/connect-proxy").unwrap();

        let agent = Agent::config_builder()
            .proxy(Some(proxy))
            .build()
            .new_agent();

        let mut res = agent.get("http://httpbin.org/get").call().unwrap();
        res.body_mut().read_to_string().unwrap();
    }

    #[test]
    fn ensure_reasonable_stack_sizes() {
        macro_rules! ensure {
            ($type:ty, $size:tt) => {
                let sz = std::mem::size_of::<$type>();
                // println!("{}: {}", stringify!($type), sz);
                assert!(
                    sz <= $size,
                    "Stack size of {} is too big {} > {}",
                    stringify!($type),
                    sz,
                    $size
                );
            };
        }

        ensure!(RequestBuilder<WithoutBody>, 400); // 304
        ensure!(Agent, 100); // 32
        ensure!(Config, 400); // 320
        ensure!(ConfigBuilder<AgentScope>, 400); // 320
        ensure!(Response<Body>, 250); // 136
        ensure!(Body, 50); // 24
    }

    #[test]
    #[cfg(feature = "_test")]
    fn limit_max_response_header_size() {
        init_test_log();
        let err = get("http://httpbin.org/get")
            .config()
            .max_response_header_size(5)
            .build()
            .call()
            .unwrap_err();
        assert!(matches!(err, Error::LargeResponseHeader(65, 5)));
    }

    #[test]
    #[cfg(feature = "_test")]
    fn propfind_with_body() {
        init_test_log();

        // https://github.com/algesten/ureq/issues/1034
        let request = http::Request::builder()
            .method("PROPFIND")
            .uri("https://www.google.com/")
            .body("Some really cool body")
            .unwrap();

        let _ = Agent::config_builder()
            .allow_non_standard_methods(true)
            .build()
            .new_agent()
            .run(request)
            .unwrap();
    }

    #[test]
    #[cfg(feature = "_test")]
    fn non_standard_method() {
        init_test_log();
        let method = Method::from_bytes(b"FNORD").unwrap();

        let req = Request::builder()
            .method(method)
            .uri("http://httpbin.org/fnord")
            .body(())
            .unwrap();

        let agent = Agent::new_with_defaults();

        let req = agent
            .configure_request(req)
            .allow_non_standard_methods(true)
            .build();

        agent.run(req).unwrap();
    }

    #[test]
    #[cfg(feature = "_test")]
    fn chunk_abort() {
        init_test_log();
        let mut res = get("http://my-fine-server/1chunk-abort").call().unwrap();
        let body = res.body_mut().read_to_string().unwrap();
        assert_eq!(body, "OK");
        let mut res = get("http://my-fine-server/2chunk-abort").call().unwrap();
        let body = res.body_mut().read_to_string().unwrap();
        assert_eq!(body, "OK");
        let mut res = get("http://my-fine-server/3chunk-abort").call().unwrap();
        let body = res.body_mut().read_to_string().unwrap();
        assert_eq!(body, "OK");
        let mut res = get("http://my-fine-server/4chunk-abort").call().unwrap();
        let body = res.body_mut().read_to_string().unwrap();
        assert_eq!(body, "OK");
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

        let err = Error::HostNotFound;
        is_send(err);
        let err = Error::HostNotFound;
        is_sync(err);
    }
}
