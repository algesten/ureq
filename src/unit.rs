use std::io::{self, Write};
use std::time;

use qstring::QString;
use url::Url;

#[cfg(feature = "cookie")]
use cookie::{Cookie, CookieJar};

use crate::agent::AgentState;
use crate::body::{self, BodySize, Payload, SizedReader};
use crate::header;
use crate::resolve::ArcResolver;
use crate::stream::{self, connect_test, Stream};
use crate::{Error, Header, Request, Response};

/// It's a "unit of work". Maybe a bad name for it?
///
/// *Internal API*
pub(crate) struct Unit {
    pub req: Request,
    pub url: Url,
    pub is_chunked: bool,
    pub query_string: String,
    pub headers: Vec<Header>,
    pub deadline: Option<time::Instant>,
}

impl Unit {
    //

    pub(crate) fn new(req: &Request, url: &Url, mix_queries: bool, body: &SizedReader) -> Self {
        //

        let (is_transfer_encoding_set, mut is_chunked) = req
            .header("transfer-encoding")
            // if the user has set an encoding header, obey that.
            .map(|enc| {
                let is_transfer_encoding_set = !enc.is_empty();
                let last_encoding = enc.split(',').last();
                let is_chunked = last_encoding
                    .map(|last_enc| last_enc.trim() == "chunked")
                    .unwrap_or(false);
                (is_transfer_encoding_set, is_chunked)
            })
            // otherwise, no chunking.
            .unwrap_or((false, false));

        let query_string = combine_query(&url, &req.query, mix_queries);

        let extra_headers = {
            let mut extra = vec![];

            // chunking and Content-Length headers are mutually exclusive
            // also don't write this if the user has set it themselves
            if !is_chunked && !req.has("content-length") {
                // if the payload is of known size (everything beside an unsized reader), set
                // Content-Length,
                // otherwise, use the chunked Transfer-Encoding (only if no other Transfer-Encoding
                // has been set
                match body.size {
                    BodySize::Known(size) => {
                        extra.push(Header::new("Content-Length", &format!("{}", size)))
                    }
                    BodySize::Unknown => {
                        if !is_transfer_encoding_set {
                            extra.push(Header::new("Transfer-Encoding", "chunked"));
                            is_chunked = true;
                        }
                    }
                    BodySize::Empty => {}
                }
            }

            let username = url.username();
            let password = url.password().unwrap_or("");
            if (username != "" || password != "") && !req.has("authorization") {
                let encoded = base64::encode(&format!("{}:{}", username, password));
                extra.push(Header::new("Authorization", &format!("Basic {}", encoded)));
            }

            #[cfg(feature = "cookie")]
            extra.extend(extract_cookies(&req.agent, &url).into_iter());

            extra
        };

        let headers: Vec<_> = req
            .headers
            .iter()
            .chain(extra_headers.iter())
            .cloned()
            .collect();

        let deadline = match req.timeout {
            None => None,
            Some(timeout) => {
                let now = time::Instant::now();
                Some(now.checked_add(timeout).unwrap())
            }
        };

        Unit {
            req: req.clone(),
            url: url.clone(),
            is_chunked,
            query_string,
            headers,
            deadline,
        }
    }

    pub fn is_head(&self) -> bool {
        self.req.method.eq_ignore_ascii_case("head")
    }

    pub fn resolver(&self) -> ArcResolver {
        self.req.agent.lock().unwrap().resolver.clone()
    }

    #[cfg(test)]
    pub fn header(&self, name: &str) -> Option<&str> {
        header::get_header(&self.headers, name)
    }
    #[cfg(test)]
    pub fn has(&self, name: &str) -> bool {
        header::has_header(&self.headers, name)
    }
    #[cfg(test)]
    pub fn all(&self, name: &str) -> Vec<&str> {
        header::get_all_headers(&self.headers, name)
    }
}

/// Perform a connection. Used recursively for redirects.
pub(crate) fn connect(
    req: &Request,
    unit: Unit,
    use_pooled: bool,
    redirect_count: u32,
    body: SizedReader,
    redir: bool,
) -> Result<Response, Error> {
    //

    let host = req.get_host()?;
    // open socket
    let (mut stream, is_recycled) = connect_socket(&unit, &host, use_pooled)?;

    let send_result = send_prelude(&unit, &mut stream, redir);

    if let Err(err) = send_result {
        if is_recycled {
            // we try open a new connection, this time there will be
            // no connection in the pool. don't use it.
            return connect(req, unit, false, redirect_count, body, redir);
        } else {
            // not a pooled connection, propagate the error.
            return Err(err.into());
        }
    }
    let retryable = req.is_retryable(&body);

    // send the body (which can be empty now depending on redirects)
    body::send_body(body, unit.is_chunked, &mut stream)?;

    // start reading the response to process cookies and redirects.
    let mut stream = stream::DeadlineStream::new(stream, unit.deadline);
    let mut resp = Response::from_read(&mut stream);

    // https://tools.ietf.org/html/rfc7230#section-6.3.1
    // When an inbound connection is closed prematurely, a client MAY
    // open a new connection and automatically retransmit an aborted
    // sequence of requests if all of those requests have idempotent
    // methods.
    //
    // We choose to retry only requests that used a recycled connection
    // from the ConnectionPool, since those are most likely to have
    // reached a server-side timeout. Note that this means we may do
    // up to N+1 total tries, where N is max_idle_connections_per_host.
    //
    // TODO: is_bad_status_read is too narrow since it covers only the
    // first line. It's also allowable to retry requests that hit a
    // closed connection during the sending or receiving of headers.
    if let Some(err) = resp.synthetic_error() {
        if err.is_bad_status_read() && retryable && is_recycled {
            let empty = Payload::Empty.into_read();
            return connect(req, unit, false, redirect_count, empty, redir);
        }
    }

    // squirrel away cookies
    #[cfg(feature = "cookie")]
    save_cookies(&unit, &resp);

    // handle redirects
    if resp.redirect() && req.redirects > 0 {
        if redirect_count == req.redirects {
            return Err(Error::TooManyRedirects);
        }

        // the location header
        let location = resp.header("location");
        if let Some(location) = location {
            // join location header to current url in case it it relative
            let new_url = unit
                .url
                .join(location)
                .map_err(|_| Error::BadUrl(format!("Bad redirection: {}", location)))?;

            // perform the redirect differently depending on 3xx code.
            match resp.status() {
                301 | 302 | 303 => {
                    let empty = Payload::Empty.into_read();
                    // recreate the unit to get a new hostname and cookies for the new host.
                    let mut new_unit = Unit::new(req, &new_url, false, &empty);
                    // this is to follow how curl does it. POST, PUT etc change
                    // to GET on a redirect.
                    new_unit.req.method = match &unit.req.method[..] {
                        "GET" | "HEAD" => unit.req.method,
                        _ => "GET".into(),
                    };
                    return connect(req, new_unit, use_pooled, redirect_count + 1, empty, true);
                }
                _ => (),
                // reinstate this with expect-100
                // 307 | 308 | _ => connect(unit, method, use_pooled, redirects - 1, body),
            };
        }
    }

    // since it is not a redirect, or we're not following redirects,
    // give away the incoming stream to the response object.
    crate::response::set_stream(&mut resp, unit.url.to_string(), Some(unit), stream.into());

    // release the response
    Ok(resp)
}

#[cfg(feature = "cookie")]
fn extract_cookies(state: &std::sync::Mutex<AgentState>, url: &Url) -> Option<Header> {
    // We specifically use url.domain() here because cookies cannot be
    // set for IP addresses.
    let domain = match url.domain() {
        Some(d) => d,
        None => return None,
    };
    let path = url.path();
    let is_secure = url.scheme().eq_ignore_ascii_case("https");

    let state = state.lock().unwrap();
    match_cookies(&state.jar, domain, path, is_secure)
}

// Return true iff the string domain-matches the domain.
// This function must only be called on hostnames, not IP addresses.
//
// https://tools.ietf.org/html/rfc6265#section-5.1.3
// A string domain-matches a given domain string if at least one of the
// following conditions hold:
//
// o  The domain string and the string are identical.  (Note that both
//    the domain string and the string will have been canonicalized to
//    lower case at this point.)
// o  All of the following conditions hold:
//    *  The domain string is a suffix of the string.
//    *  The last character of the string that is not included in the
//       domain string is a %x2E (".") character.
//    *  The string is a host name (i.e., not an IP address).
#[cfg(feature = "cookie")]
fn domain_match(s: &str, domain: &str) -> bool {
    match s.strip_suffix(domain) {
        Some("") => true, // domain and string are identical.
        Some(remains) => remains.ends_with('.'),
        None => false, // domain was not a suffix of string.
    }
}

// Return true iff the request-path path-matches the cookie-path.
// https://tools.ietf.org/html/rfc6265#section-5.1.4
//  A request-path path-matches a given cookie-path if at least one of
//  the following conditions holds:
//
//  o  The cookie-path and the request-path are identical.
//  o  The cookie-path is a prefix of the request-path, and the last
//       character of the cookie-path is %x2F ("/").
//  o  The cookie-path is a prefix of the request-path, and the first
//       character of the request-path that is not included in the cookie-
//       path is a %x2F ("/") character.
#[cfg(feature = "cookie")]
fn path_match(request_path: &str, cookie_path: &str) -> bool {
    match request_path.strip_prefix(cookie_path) {
        Some("") => true, // cookie path and request path were identical.
        Some(remains) => cookie_path.ends_with('/') || remains.starts_with('/'),
        None => false, // cookie path was not a prefix of request path
    }
}

#[cfg(feature = "cookie")]
fn match_cookies(jar: &CookieJar, domain: &str, path: &str, is_secure: bool) -> Option<Header> {
    let header_value = jar
        .iter()
        .filter(|c| domain_match(domain, c.domain().unwrap()))
        .filter(|c| path_match(path, c.path().unwrap()))
        .filter(|c| is_secure || !c.secure().unwrap_or(false))
        // Create a new cookie with just the name and value so we don't send attributes.
        .map(|c| Cookie::new(c.name(), c.value()).encoded().to_string())
        .collect::<Vec<_>>()
        .join(";");
    match header_value.as_str() {
        "" => None,
        val => Some(Header::new("Cookie", val)),
    }
}

/// Combine the query of the url and the query options set on the request object.
pub(crate) fn combine_query(url: &Url, query: &QString, mix_queries: bool) -> String {
    match (url.query(), !query.is_empty() && mix_queries) {
        (Some(urlq), true) => format!("?{}&{}", urlq, query),
        (Some(urlq), false) => format!("?{}", urlq),
        (None, true) => format!("?{}", query),
        (None, false) => "".to_string(),
    }
}

/// Connect the socket, either by using the pool or grab a new one.
fn connect_socket(unit: &Unit, hostname: &str, use_pooled: bool) -> Result<(Stream, bool), Error> {
    match unit.url.scheme() {
        "http" | "https" | "test" => (),
        _ => return Err(Error::UnknownScheme(unit.url.scheme().to_string())),
    };
    if use_pooled {
        let state = &mut unit.req.agent.lock().unwrap();
        // The connection may have been closed by the server
        // due to idle timeout while it was sitting in the pool.
        // Loop until we find one that is still good or run out of connections.
        while let Some(stream) = state.pool.try_get_connection(&unit.url, &unit.req.proxy) {
            let server_closed = stream.server_closed()?;
            if !server_closed {
                return Ok((stream, true));
            }
        }
    }
    let stream = match unit.url.scheme() {
        "http" => stream::connect_http(&unit, hostname),
        "https" => stream::connect_https(&unit, hostname),
        "test" => connect_test(&unit),
        _ => Err(Error::UnknownScheme(unit.url.scheme().to_string())),
    };
    Ok((stream?, false))
}

/// Send request line + headers (all up until the body).
#[allow(clippy::write_with_newline)]
fn send_prelude(unit: &Unit, stream: &mut Stream, redir: bool) -> io::Result<()> {
    //

    // build into a buffer and send in one go.
    let mut prelude: Vec<u8> = vec![];

    // request line
    write!(
        prelude,
        "{} {}{} HTTP/1.1\r\n",
        unit.req.method,
        unit.url.path(),
        &unit.query_string
    )?;

    // host header if not set by user.
    if !header::has_header(&unit.headers, "host") {
        let host = unit.url.host().unwrap();
        match unit.url.port() {
            Some(port) => {
                let scheme_default: u16 = match unit.url.scheme() {
                    "http" => 80,
                    "https" => 443,
                    _ => 0,
                };
                if scheme_default != 0 && scheme_default == port {
                    write!(prelude, "Host: {}\r\n", host)?;
                } else {
                    write!(prelude, "Host: {}:{}\r\n", host, port)?;
                }
            }
            None => {
                write!(prelude, "Host: {}\r\n", host)?;
            }
        }
    }
    if !header::has_header(&unit.headers, "user-agent") {
        write!(
            prelude,
            "User-Agent: ureq/{}\r\n",
            env!("CARGO_PKG_VERSION")
        )?;
    }
    if !header::has_header(&unit.headers, "accept") {
        write!(prelude, "Accept: */*\r\n")?;
    }

    // other headers
    for header in &unit.headers {
        if !redir || !header.is_name("Authorization") {
            write!(prelude, "{}: {}\r\n", header.name(), header.value())?;
        }
    }

    // finish
    write!(prelude, "\r\n")?;

    // write all to the wire
    stream.write_all(&prelude[..])?;

    Ok(())
}

/// Investigate a response for "Set-Cookie" headers.
#[cfg(feature = "cookie")]
fn save_cookies(unit: &Unit, resp: &Response) {
    //

    // Specifically use domain here because IPs cannot have cookies.
    let request_domain = match unit.url.domain() {
        Some(d) => d.to_ascii_lowercase(),
        None => return,
    };
    let headers = resp.all("set-cookie");
    // Avoid locking if there are no cookie headers
    if headers.is_empty() {
        return;
    }
    let cookies = headers.into_iter().flat_map(|header_value| {
        let mut cookie = match Cookie::parse_encoded(header_value) {
            Err(_) => return None,
            Ok(c) => c,
        };
        // Canonicalize the cookie domain, check that it matches the request,
        // and store it back in the cookie.
        // https://tools.ietf.org/html/rfc6265#section-5.3, Item 6
        // Summary: If domain is empty, set it from the request and
        // set the host_only flag.
        // TODO: store a host_only flag.
        // TODO: Check so cookies can't be set for TLDs.
        let cookie_domain = match cookie.domain() {
            None => request_domain.clone(),
            Some(d) if domain_match(&request_domain, &d) => d.to_ascii_lowercase(),
            Some(_) => return None,
        };
        cookie.set_domain(cookie_domain);
        if cookie.path().is_none() {
            cookie.set_path("/");
        }
        Some(cookie)
    });
    let state = &mut unit.req.agent.lock().unwrap();
    for c in cookies {
        assert!(c.domain().is_some());
        assert!(c.path().is_some());
        state.jar.add(c.into_owned());
    }
}

#[cfg(test)]
#[cfg(feature = "cookies")]
mod tests {
    use super::*;

    ///////////////////// COOKIE TESTS //////////////////////////////

    #[test]
    fn match_cookies_returns_nothing_when_no_cookies() {
        let jar = CookieJar::new();

        let result = match_cookies(&jar, "crates.io", "/", false);
        assert_eq!(result, None);
    }

    #[test]
    fn match_cookies_returns_one_header() {
        let mut jar = CookieJar::new();
        let cookie1 = Cookie::parse("cookie1=value1; Domain=crates.io; Path=/").unwrap();
        let cookie2 = Cookie::parse("cookie2=value2; Domain=crates.io; Path=/").unwrap();
        jar.add(cookie1);
        jar.add(cookie2);

        // There's no guarantee to the order in which cookies are defined.
        // Ensure that they're either in one order or the other.
        let result = match_cookies(&jar, "crates.io", "/", false);
        let order1 = "cookie1=value1;cookie2=value2";
        let order2 = "cookie2=value2;cookie1=value1";

        assert!(
            result == Some(Header::new("Cookie", order1))
                || result == Some(Header::new("Cookie", order2))
        );
    }
}
