use std::io::{self, Write};
use std::time;

use log::{debug, info};
use url::Url;

#[cfg(feature = "cookies")]
use cookie::Cookie;

use crate::body::{self, BodySize, Payload, SizedReader};
use crate::error::{Error, ErrorKind};
use crate::header;
use crate::header::{get_header, Header};
use crate::resolve::ArcResolver;
use crate::response::Response;
use crate::stream::{self, connect_test, Stream};
use crate::Agent;

/// A Unit is fully-built Request, ready to execute.
///
/// *Internal API*
#[derive(Clone)]
pub(crate) struct Unit {
    pub agent: Agent,
    pub method: String,
    pub url: Url,
    is_chunked: bool,
    headers: Vec<Header>,
    pub deadline: Option<time::Instant>,
}

impl Unit {
    //

    pub(crate) fn new(
        agent: &Agent,
        method: &str,
        url: &Url,
        headers: &Vec<Header>,
        body: &SizedReader,
        deadline: Option<time::Instant>,
    ) -> Self {
        //

        let (is_transfer_encoding_set, mut is_chunked) = get_header(&headers, "transfer-encoding")
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

        let extra_headers = {
            let mut extra = vec![];

            // chunking and Content-Length headers are mutually exclusive
            // also don't write this if the user has set it themselves
            if !is_chunked && get_header(&headers, "content-length").is_none() {
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
            if (!username.is_empty() || !password.is_empty())
                && get_header(&headers, "authorization").is_none()
            {
                let encoded = base64::encode(&format!("{}:{}", username, password));
                extra.push(Header::new("Authorization", &format!("Basic {}", encoded)));
            }

            #[cfg(feature = "cookies")]
            extra.extend(extract_cookies(agent, &url).into_iter());

            extra
        };

        let headers: Vec<_> = headers
            .iter()
            .chain(extra_headers.iter())
            .cloned()
            .collect();

        Unit {
            agent: agent.clone(),
            method: method.to_string(),
            url: url.clone(),
            is_chunked,
            headers,
            deadline,
        }
    }

    pub fn is_head(&self) -> bool {
        self.method.eq_ignore_ascii_case("head")
    }

    pub fn resolver(&self) -> ArcResolver {
        self.agent.state.resolver.clone()
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

    // Returns true if this request, with the provided body, is retryable.
    pub(crate) fn is_retryable(&self, body: &SizedReader) -> bool {
        // Per https://tools.ietf.org/html/rfc7231#section-8.1.3
        // these methods are idempotent.
        let idempotent = match self.method.as_str() {
            "DELETE" | "GET" | "HEAD" | "OPTIONS" | "PUT" | "TRACE" => true,
            _ => false,
        };
        // Unsized bodies aren't retryable because we can't rewind the reader.
        // Sized bodies are retryable only if they are zero-length because of
        // coincidences of the current implementation - the function responsible
        // for retries doesn't have a way to replay a Payload.
        let retryable_body = match body.size {
            BodySize::Unknown => false,
            BodySize::Known(0) => true,
            BodySize::Known(_) => false,
            BodySize::Empty => true,
        };

        idempotent && retryable_body
    }
}

/// Perform a connection. Follows redirects.
pub(crate) fn connect(
    mut unit: Unit,
    use_pooled: bool,
    mut body: SizedReader,
) -> Result<Response, Error> {
    let mut history = vec![];
    let mut resp = loop {
        let resp = connect_inner(&unit, use_pooled, body, &history)?;

        // handle redirects
        if !(300..399).contains(&resp.status()) || unit.agent.config.redirects == 0 {
            break resp;
        }
        if history.len() + 1 >= unit.agent.config.redirects as usize {
            return Err(ErrorKind::TooManyRedirects.new());
        }
        // the location header
        let location = match resp.header("location") {
            Some(l) => l,
            None => break resp,
        };

        let url = &unit.url;
        let method = &unit.method;
        // join location header to current url in case it is relative
        let new_url = url.join(location).map_err(|e| {
            ErrorKind::InvalidUrl
                .msg(&format!("Bad redirection: {}", location))
                .src(e)
        })?;

        // perform the redirect differently depending on 3xx code.
        let new_method = match resp.status() {
            // this is to follow how curl does it. POST, PUT etc change
            // to GET on a redirect.
            301 | 302 | 303 => match &method[..] {
                "GET" | "HEAD" => unit.method,
                _ => "GET".into(),
            },
            // never change the method for 307/308
            // only resend the request if it cannot have a body
            // NOTE: DELETE is intentionally excluded: https://stackoverflow.com/questions/299628
            307 | 308 if ["GET", "HEAD", "OPTIONS", "TRACE"].contains(&method.as_str()) => {
                unit.method
            }
            _ => break resp,
        };
        debug!("redirect {} {} -> {}", resp.status(), url, new_url);
        history.push(unit.url.to_string());
        body = Payload::Empty.into_read();
        // recreate the unit to get a new hostname and cookies for the new host.
        unit = Unit::new(
            &unit.agent,
            &new_method,
            &new_url,
            &unit.headers,
            &body,
            unit.deadline,
        );
    };
    resp.history = history;
    Ok(resp)
}

/// Perform a connection. Does not follow redirects.
fn connect_inner(
    unit: &Unit,
    use_pooled: bool,
    body: SizedReader,
    previous: &[String],
) -> Result<Response, Error> {
    let host = unit
        .url
        .host_str()
        .ok_or(ErrorKind::InvalidUrl.msg("no host in URL"))?;
    let url = &unit.url;
    let method = &unit.method;
    // open socket
    let (mut stream, is_recycled) = connect_socket(&unit, &host, use_pooled)?;

    if is_recycled {
        info!("sending request (reused connection) {} {}", method, url);
    } else {
        info!("sending request {} {}", method, url);
    }

    let send_result = send_prelude(&unit, &mut stream, !previous.is_empty());

    if let Err(err) = send_result {
        if is_recycled {
            debug!("retrying request early {} {}: {}", method, url, err);
            // we try open a new connection, this time there will be
            // no connection in the pool. don't use it.
            // NOTE: this recurses at most once because `use_pooled` is `false`.
            return connect_inner(unit, false, body, previous);
        } else {
            // not a pooled connection, propagate the error.
            return Err(err.into());
        }
    }
    let retryable = unit.is_retryable(&body);

    // send the body (which can be empty now depending on redirects)
    body::send_body(body, unit.is_chunked, &mut stream)?;

    // start reading the response to process cookies and redirects.
    let result = Response::do_from_request(unit.clone(), stream);

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
    let resp = match result {
        Err(err) if err.connection_closed() && retryable && is_recycled => {
            debug!("retrying request {} {}: {}", method, url, err);
            let empty = Payload::Empty.into_read();
            // NOTE: this recurses at most once because `use_pooled` is `false`.
            return connect_inner(unit, false, empty, previous);
        }
        Err(e) => return Err(e),
        Ok(resp) => resp,
    };

    // squirrel away cookies
    #[cfg(feature = "cookies")]
    save_cookies(&unit, &resp);

    debug!("response {} to {} {}", resp.status(), method, url);

    // release the response
    Ok(resp)
}

#[cfg(feature = "cookies")]
fn extract_cookies(agent: &Agent, url: &Url) -> Option<Header> {
    let header_value = agent
        .state
        .cookie_tin
        .get_request_cookies(url)
        .iter()
        .map(|c| c.encoded().to_string())
        .collect::<Vec<_>>()
        .join(";");
    match header_value.as_str() {
        "" => None,
        val => Some(Header::new("Cookie", val)),
    }
}

/// Connect the socket, either by using the pool or grab a new one.
fn connect_socket(unit: &Unit, hostname: &str, use_pooled: bool) -> Result<(Stream, bool), Error> {
    match unit.url.scheme() {
        "http" | "https" | "test" => (),
        scheme => return Err(ErrorKind::UnknownScheme.msg(&format!("unknown scheme '{}'", scheme))),
    };
    if use_pooled {
        let pool = &unit.agent.state.pool;
        let proxy = &unit.agent.config.proxy;
        // The connection may have been closed by the server
        // due to idle timeout while it was sitting in the pool.
        // Loop until we find one that is still good or run out of connections.
        while let Some(stream) = pool.try_get_connection(&unit.url, proxy.clone()) {
            let server_closed = stream.server_closed()?;
            if !server_closed {
                return Ok((stream, true));
            }
            debug!("dropping stream from pool; closed by server: {:?}", stream);
        }
    }
    let stream = match unit.url.scheme() {
        "http" => stream::connect_http(&unit, hostname),
        "https" => stream::connect_https(&unit, hostname),
        "test" => connect_test(&unit),
        scheme => Err(ErrorKind::UnknownScheme.msg(&format!("unknown scheme {}", scheme))),
    };
    Ok((stream?, false))
}

/// Send request line + headers (all up until the body).
#[allow(clippy::write_with_newline)]
fn send_prelude(unit: &Unit, stream: &mut Stream, redir: bool) -> io::Result<()> {
    // build into a buffer and send in one go.
    let mut prelude: Vec<u8> = vec![];

    // request line
    write!(
        prelude,
        "{} {}{}{} HTTP/1.1\r\n",
        unit.method,
        unit.url.path(),
        if unit.url.query().is_some() { "?" } else { "" },
        unit.url.query().unwrap_or_default(),
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
        write!(prelude, "User-Agent: {}\r\n", &unit.agent.config.user_agent)?;
    }
    if !header::has_header(&unit.headers, "accept") {
        write!(prelude, "Accept: */*\r\n")?;
    }

    // other headers
    for header in &unit.headers {
        if !redir || !header.is_name("Authorization") {
            if let Some(v) = header.value() {
                write!(prelude, "{}: {}\r\n", header.name(), v)?;
            }
        }
    }

    // finish
    write!(prelude, "\r\n")?;

    debug!("writing prelude: {}", String::from_utf8_lossy(&prelude));
    // write all to the wire
    stream.write_all(&prelude[..])?;

    Ok(())
}

/// Investigate a response for "Set-Cookie" headers.
#[cfg(feature = "cookies")]
fn save_cookies(unit: &Unit, resp: &Response) {
    //

    let headers = resp.all("set-cookie");
    // Avoid locking if there are no cookie headers
    if headers.is_empty() {
        return;
    }
    let cookies = headers.into_iter().flat_map(|header_value| {
        debug!(
            "received 'set-cookie: {}' from {} {}",
            header_value, unit.method, unit.url
        );
        match Cookie::parse(header_value.to_string()) {
            Err(_) => None,
            Ok(c) => Some(c),
        }
    });
    unit.agent
        .state
        .cookie_tin
        .store_response_cookies(cookies, &unit.url.clone());
}

#[cfg(test)]
#[cfg(feature = "cookies")]
mod tests {
    use super::*;

    use crate::Agent;
    ///////////////////// COOKIE TESTS //////////////////////////////

    #[test]
    fn match_cookies_returns_one_header() {
        let agent = Agent::new();
        let url: Url = "https://crates.io/".parse().unwrap();
        let cookie1: Cookie = "cookie1=value1; Domain=crates.io; Path=/".parse().unwrap();
        let cookie2: Cookie = "cookie2=value2; Domain=crates.io; Path=/".parse().unwrap();
        agent
            .state
            .cookie_tin
            .store_response_cookies(vec![cookie1, cookie2].into_iter(), &url);

        // There's no guarantee to the order in which cookies are defined.
        // Ensure that they're either in one order or the other.
        let result = extract_cookies(&agent, &url);
        let order1 = "cookie1=value1;cookie2=value2";
        let order2 = "cookie2=value2;cookie1=value1";

        assert!(
            result == Some(Header::new("Cookie", order1))
                || result == Some(Header::new("Cookie", order2))
        );
    }
}
