use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex};
use std::time;

use qstring::QString;
use url::Url;

#[cfg(feature = "cookie")]
use cookie::{Cookie, CookieJar};

use crate::agent::AgentState;
use crate::body::{self, Payload, SizedReader};
use crate::header;
use crate::stream::{self, connect_test, Stream};
use crate::Proxy;
use crate::{Error, Header, Request, Response};

#[cfg(feature = "tls")]
use crate::request::TLSClientConfig;

#[cfg(all(feature = "native-tls", not(feature = "tls")))]
use crate::request::TLSConnector;

#[cfg(feature = "cookie")]
use crate::pool::DEFAULT_HOST;

/// It's a "unit of work". Maybe a bad name for it?
///
/// *Internal API*
#[derive(Debug)]
pub(crate) struct Unit {
    pub agent: Arc<Mutex<Option<AgentState>>>,
    pub url: Url,
    pub is_chunked: bool,
    pub query_string: String,
    pub headers: Vec<Header>,
    pub timeout_connect: u64,
    pub timeout_read: u64,
    pub timeout_write: u64,
    pub deadline: Option<time::Instant>,
    pub method: String,
    pub proxy: Option<Proxy>,
    #[cfg(feature = "tls")]
    pub tls_config: Option<TLSClientConfig>,
    #[cfg(all(feature = "native-tls", not(feature = "tls")))]
    pub tls_connector: Option<TLSConnector>,
}

impl Unit {
    //

    pub(crate) fn new(req: &Request, url: &Url, mix_queries: bool, body: &SizedReader) -> Self {
        //

        let is_chunked = req
            .header("transfer-encoding")
            // if the user has set an encoding header, obey that.
            .map(|enc| !enc.is_empty())
            // otherwise, no chunking.
            .unwrap_or(false);

        let query_string = combine_query(&url, &req.query, mix_queries);

        let cookie_header: Option<Header> = extract_cookies(&req.agent, &url);

        let extra_headers = {
            let mut extra = vec![];

            // chunking and Content-Length headers are mutually exclusive
            // also don't write this if the user has set it themselves
            if !is_chunked && !req.has("content-length") {
                if let Some(size) = body.size {
                    extra.push(Header::new("Content-Length", &format!("{}", size)));
                }
            }

            let username = url.username();
            let password = url.password().unwrap_or("");
            if (username != "" || password != "") && !req.has("authorization") {
                let encoded = base64::encode(&format!("{}:{}", username, password));
                extra.push(Header::new("Authorization", &format!("Basic {}", encoded)));
            }

            extra
        };
        let headers: Vec<_> = req
            .headers
            .iter()
            .chain(cookie_header.iter())
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
            agent: Arc::clone(&req.agent),
            url: url.clone(),
            is_chunked,
            query_string,
            headers,
            timeout_connect: req.timeout_connect,
            timeout_read: req.timeout_read,
            timeout_write: req.timeout_write,
            deadline,
            method: req.method.clone(),
            proxy: req.proxy.clone(),
            #[cfg(feature = "tls")]
            tls_config: req.tls_config.clone(),
            #[cfg(all(feature = "native-tls", not(feature = "tls")))]
            tls_connector: req.tls_connector.clone(),
        }
    }

    pub fn is_head(&self) -> bool {
        self.method.eq_ignore_ascii_case("head")
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

    // open socket
    let (mut stream, is_recycled) = connect_socket(&unit, use_pooled)?;

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
    // We choose to retry only once. To do that, we rely on is_recycled,
    // the "one connection per hostname" police of the ConnectionPool,
    // and the fact that connections with errors are dropped.
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
                    new_unit.method = match &unit.method[..] {
                        "GET" | "HEAD" => unit.method,
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
fn extract_cookies(state: &std::sync::Mutex<Option<AgentState>>, url: &Url) -> Option<Header> {
    let state = state.lock().unwrap();
    let is_secure = url.scheme().eq_ignore_ascii_case("https");
    let hostname = url.host_str().unwrap_or(DEFAULT_HOST).to_string();

    state
        .as_ref()
        .map(|state| &state.jar)
        .and_then(|jar| match_cookies(jar, &hostname, url.path(), is_secure))
}

#[cfg(not(feature = "cookie"))]
fn extract_cookies(_state: &std::sync::Mutex<Option<AgentState>>, _url: &Url) -> Option<Header> {
    None
}

// TODO check so cookies can't be set for tld:s
#[cfg(feature = "cookie")]
fn match_cookies(jar: &CookieJar, domain: &str, path: &str, is_secure: bool) -> Option<Header> {
    Some(
        jar.iter()
            .filter(|c| {
                // if there is a domain, it must be matched.
                // if there is no domain, then ignore cookie
                let domain_ok = c
                    .domain()
                    .map(|cdom| domain.contains(cdom))
                    .unwrap_or(false);
                // a path must match the beginning of request path.
                // no cookie path, we say is ok. is it?!
                let path_ok = c
                    .path()
                    .map(|cpath| path.find(cpath).map(|pos| pos == 0).unwrap_or(false))
                    .unwrap_or(true);
                // either the cookie isnt secure, or we're not doing a secure request.
                let secure_ok = !c.secure().unwrap_or(false) || is_secure;

                domain_ok && path_ok && secure_ok
            })
            .map(|c| {
                let name = c.name().to_string();
                let value = c.value().to_string();
                let nameval = Cookie::new(name, value).encoded().to_string();
                nameval
            })
            .collect::<Vec<_>>()
            .join(";"),
    )
    .filter(|x| !x.is_empty())
    .map(|s| Header::new("Cookie", &s))
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
fn connect_socket(unit: &Unit, use_pooled: bool) -> Result<(Stream, bool), Error> {
    match unit.url.scheme() {
        "http" | "https" | "test" => (),
        _ => return Err(Error::UnknownScheme(unit.url.scheme().to_string())),
    };
    if use_pooled {
        let state = &mut unit.agent.lock().unwrap();
        if let Some(agent) = state.as_mut() {
            // The connection may have been closed by the server
            // due to idle timeout while it was sitting in the pool.
            // Loop until we find one that is still good or run out of connections.
            while let Some(stream) = agent.pool.try_get_connection(&unit.url, &unit.proxy) {
                let server_closed = stream.server_closed()?;
                if !server_closed {
                    return Ok((stream, true));
                }
            }
        }
    }
    let stream = match unit.url.scheme() {
        "http" => stream::connect_http(&unit),
        "https" => stream::connect_https(&unit),
        "test" => connect_test(&unit),
        _ => Err(Error::UnknownScheme(unit.url.scheme().to_string())),
    };
    Ok((stream?, false))
}

/// Send request line + headers (all up until the body).
#[allow(clippy::write_with_newline)]
fn send_prelude(unit: &Unit, stream: &mut Stream, redir: bool) -> IoResult<()> {
    //

    // build into a buffer and send in one go.
    let mut prelude: Vec<u8> = vec![];

    // request line
    write!(
        prelude,
        "{} {}{} HTTP/1.1\r\n",
        unit.method,
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
        write!(prelude, "User-Agent: ureq\r\n")?;
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

#[cfg(not(feature = "cookie"))]
fn save_cookies(_unit: &Unit, _resp: &Response) {}

/// Investigate a response for "Set-Cookie" headers.
#[cfg(feature = "cookie")]
fn save_cookies(unit: &Unit, resp: &Response) {
    //

    let cookies = resp.all("set-cookie");
    if cookies.is_empty() {
        return;
    }

    // only lock if we know there is something to process
    let state = &mut unit.agent.lock().unwrap();
    if let Some(add_jar) = state.as_mut().map(|state| &mut state.jar) {
        for raw_cookie in cookies.iter() {
            let to_parse = if raw_cookie.to_lowercase().contains("domain=") {
                (*raw_cookie).to_string()
            } else {
                let host = &unit.url.host_str().unwrap_or(DEFAULT_HOST).to_string();
                format!("{}; Domain={}", raw_cookie, host)
            };
            match Cookie::parse_encoded(&to_parse[..]) {
                Err(_) => (), // ignore unparseable cookies
                Ok(cookie) => {
                    let cookie = cookie.into_owned();
                    add_jar.add(cookie)
                }
            }
        }
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
        let cookie1 = Cookie::parse("cookie1=value1; Domain=crates.io").unwrap();
        let cookie2 = Cookie::parse("cookie2=value2; Domain=crates.io").unwrap();
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
