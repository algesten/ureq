//! ureq is a minimal request library.
//!
//! The goals of this library are:
//!
//! * Minimal dependency tree
//! * Obvious API
//!
//! # Plain requests
//!
//! Most standard methods (GET, POST, PUT etc), are supported as functions from the
//! top of the library ([`ureq::get`](fn.get.html), [`ureq::post`](fn.post.html),
//! [`ureq::put`](fn.out.html), etc).
//!
//! These top level http method functions create a [Request](struct.Request.html) instance
//! which follows a build pattern. The builders are finished using
//! [`.call()`](struct.Request.html#method.call),
//! [`.send_str()`](struct.Request.html#method.send_str) or
//! [`.send_json()`](struct.Request.html#method.send_json).
//!
//! # Agents
//!
//! To maintain a state, cookies, between requests, you use an [agent](struct.Agent.html).
//! Agents also follow the build pattern. Agents are created with `ureq::agent().build()`.
//!

extern crate ascii;
extern crate base64;
extern crate chunked_transfer;
extern crate cookie;
extern crate dns_lookup;
extern crate encoding;
#[macro_use]
extern crate lazy_static;
extern crate mime_guess;
extern crate qstring;
extern crate serde_json;
extern crate native_tls;
extern crate url;

mod agent;
mod error;
mod header;
mod macros;
mod serde_macros;

#[cfg(test)]
mod test;

pub use agent::{Agent, Request, Response};
pub use header::Header;

// re-export
pub use serde_json::{to_value as serde_to_value, Map as SerdeMap, Value as SerdeValue};
pub use cookie::Cookie;

/// Agents are used to keep state between requests.
pub fn agent() -> Agent {
    Agent::new()
}

/// Make a request setting the HTTP method via a string.
///
/// ```
/// ureq::request("GET", "https://www.google.com").call();
/// ```
pub fn request<M, S>(method: M, path: S) -> Request
where
    M: Into<String>,
    S: Into<String>,
{
    Agent::new().request(method, path)
}

/// Make a GET request.
pub fn get<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("GET", path)
}

/// Make a HEAD request.
pub fn head<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("HEAD", path)
}

/// Make a POST request.
pub fn post<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("POST", path)
}

/// Make a PUT request.
pub fn put<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("PUT", path)
}

/// Make a DELETE request.
pub fn delete<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("DELETE", path)
}

/// Make a TRACE request.
pub fn trace<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("TRACE", path)
}

/// Make an OPTIONS request.
pub fn options<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("OPTIONS", path)
}

/// Make an CONNECT request.
pub fn connect<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("CONNECT", path)
}

/// Make an PATCH request.
pub fn patch<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("PATCH", path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_http_google() {
        let resp = get("http://www.google.com/").call();
        assert_eq!(
            "text/html; charset=ISO-8859-1",
            resp.header("content-type").unwrap()
        );
        assert_eq!("text/html", resp.content_type());
    }

    #[test]
    fn connect_https_google() {
        let resp = get("https://www.google.com/").call();
        assert_eq!(
            "text/html; charset=ISO-8859-1",
            resp.header("content-type").unwrap()
        );
        assert_eq!("text/html", resp.content_type());
    }
}
