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
pub use serde_json::{to_value, Map, Value};
pub use cookie::Cookie;

/// Agents keep state between requests.
///
/// By default, no state, such as cookies, is kept between requests.
/// But by creating an agent as entry point for the request, we
/// can keep state.
///
/// ```
/// let agent = ureq::agent();
///
/// let auth = agent
///     .post("/login")
///     .auth("martin", "rubbermashgum")
///     .call(); // blocks. puts auth cookies in agent.
///
/// if !auth.ok() {
///     println!("Noes!");
/// }
///
/// let secret = agent
///     .get("/my-protected-page")
///     .call(); // blocks and waits for request.
///
/// if !secret.ok() {
///     println!("Wot?!");
/// }
///
/// println!("Secret is: {}", secret.into_string().unwrap());
/// ```
pub fn agent() -> Agent {
    Agent::new()
}

pub fn request<M, S>(method: M, path: S) -> Request
where
    M: Into<String>,
    S: Into<String>,
{
    Agent::new().request(method, path)
}

pub fn get<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("GET", path)
}
pub fn head<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("HEAD", path)
}
pub fn post<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("POST", path)
}
pub fn put<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("PUT", path)
}
pub fn delete<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("DELETE", path)
}
pub fn trace<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("TRACE", path)
}
pub fn options<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("OPTIONS", path)
}
pub fn connect<S>(path: S) -> Request
where
    S: Into<String>,
{
    request("CONNECT", path)
}
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
