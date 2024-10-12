use std::convert::TryFrom;
use std::sync::Arc;
use std::{io, mem};

use hoot::client::flow::state::{Await100, RecvBody, RecvResponse, Redirect, SendRequest};
use hoot::client::flow::state::{Prepare, SendBody as SendBodyState};
use hoot::client::flow::{Await100Result, RecvBodyResult, RecvResponseResult, SendRequestResult};
use hoot::BodyMode;
use http::uri::Scheme;
use http::{HeaderValue, Request, Response, Uri};

use crate::body::ResponseInfo;
use crate::config::{Config, RequestLevelConfig};
use crate::pool::Connection;
use crate::timings::{CallTimings, CurrentTime};
use crate::transport::time::{Duration, Instant};
use crate::transport::ConnectionDetails;
use crate::util::{DebugRequest, DebugResponse, DebugUri, HeaderMapExt, UriExt};
use crate::{Agent, Body, Error, SendBody, Timeout};

type Flow<T> = hoot::client::flow::Flow<(), T>;

/// Run a request.
///
/// This is the "main loop" of entire ureq.
pub(crate) fn run(
    agent: &Agent,
    mut request: Request<()>,
    mut body: SendBody,
) -> Result<Response<Body>, Error> {
    let mut redirect_count = 0;

    // Configuration on the request level overrides the agent level.
    let config = request
        .extensions_mut()
        .remove::<RequestLevelConfig>()
        .map(|rl| rl.0)
        .map(Arc::new)
        .unwrap_or_else(|| agent.config.clone());

    let timeouts = config.timeouts;

    let mut timings = CallTimings::new(timeouts, CurrentTime::default());

    let mut flow = Flow::new(request)?;

    let (response, handler) = loop {
        let timeout = timings.next_timeout(Timeout::Global);
        let timed_out = match timeout.after {
            Duration::Exact(v) => v.is_zero(),
            Duration::NotHappening => false,
        };
        if timed_out {
            return Err(Error::Timeout(Timeout::Global));
        }

        match flow_run(
            agent,
            &config,
            flow,
            &mut body,
            redirect_count,
            &mut timings,
        )? {
            // Follow redirect
            FlowResult::Redirect(rflow, rtimings) => {
                redirect_count += 1;

                flow = handle_redirect(rflow, &config)?;
                timings = rtimings.new_call();
            }

            // Return response
            FlowResult::Response(response, handler) => break (response, handler),
        }
    };

    let (parts, _) = response.into_parts();

    let recv_body_mode = handler
        .flow
        .as_ref()
        .map(|f| f.body_mode())
        .unwrap_or(BodyMode::NoBody);

    let info = ResponseInfo::new(&parts.headers, recv_body_mode);

    let body = Body::new(handler, info);

    let response = Response::from_parts(parts, body);

    let status = response.status();
    let is_err = status.is_client_error() || status.is_server_error();

    if config.http_status_as_error && is_err {
        return Err(Error::StatusCode(status.as_u16()));
    }

    Ok(response)
}

fn flow_run(
    agent: &Agent,
    config: &Config,
    mut flow: Flow<Prepare>,
    body: &mut SendBody,
    redirect_count: u32,
    timings: &mut CallTimings,
) -> Result<FlowResult, Error> {
    let uri = flow.uri().clone();
    debug!("{} {:?}", flow.method(), &DebugUri(flow.uri()));

    if config.https_only && uri.scheme() != Some(&Scheme::HTTPS) {
        return Err(Error::RequireHttpsOnly(uri.to_string()));
    }

    add_headers(&mut flow, agent, config, body, &uri)?;

    let mut connection = connect(agent, config, &uri, timings)?;

    let mut flow = flow.proceed();

    if log_enabled!(log::Level::Debug) {
        let headers = flow.headers_map()?;

        let r = DebugRequest {
            method: flow.method(),
            uri: flow.uri(),
            version: flow.version(),
            headers,
        };
        debug!("{:?}", r);
    }

    let flow = match send_request(flow, &mut connection, timings)? {
        SendRequestResult::Await100(flow) => match await_100(flow, &mut connection, timings)? {
            Await100Result::SendBody(flow) => send_body(flow, body, &mut connection, timings)?,
            Await100Result::RecvResponse(flow) => flow,
        },
        SendRequestResult::SendBody(flow) => send_body(flow, body, &mut connection, timings)?,
        SendRequestResult::RecvResponse(flow) => flow,
    };

    let (response, response_result) = recv_response(flow, &mut connection, config, timings)?;

    debug!("{:?}", DebugResponse(&response));

    let ret = match response_result {
        RecvResponseResult::RecvBody(flow) => {
            let timings = mem::take(timings);
            let mut handler = BodyHandler {
                flow: Some(flow),
                connection: Some(connection),
                timings,
                ..Default::default()
            };

            if response.status().is_redirection() && redirect_count < config.max_redirects {
                let flow = handler.consume_redirect_body()?;

                FlowResult::Redirect(flow, handler.timings)
            } else {
                FlowResult::Response(response, handler)
            }
        }
        RecvResponseResult::Redirect(flow) => {
            cleanup(connection, flow.must_close_connection(), timings.now());

            if redirect_count >= config.max_redirects {
                FlowResult::Response(response, BodyHandler::default())
            } else {
                FlowResult::Redirect(flow, mem::take(timings))
            }
        }
        RecvResponseResult::Cleanup(flow) => {
            cleanup(connection, flow.must_close_connection(), timings.now());
            FlowResult::Response(response, BodyHandler::default())
        }
    };

    Ok(ret)
}

/// Return type of [`flow_run`].
#[allow(clippy::large_enum_variant)]
enum FlowResult {
    /// Flow resulted in a redirect.
    Redirect(Flow<Redirect>, CallTimings),

    /// Flow resulted in a response.
    Response(Response<()>, BodyHandler),
}

fn add_headers(
    flow: &mut Flow<Prepare>,
    agent: &Agent,
    config: &Config,
    body: &SendBody,
    uri: &Uri,
) -> Result<(), Error> {
    let headers = flow.headers();

    let send_body_mode = if headers.has_send_body_mode() {
        None
    } else {
        Some(body.body_mode())
    };
    #[cfg(any(feature = "gzip", feature = "brotli"))]
    let has_header_accept_enc = headers.has_accept_encoding();
    let has_header_ua = headers.has_user_agent();
    let has_header_accept = headers.has_accept();

    #[cfg(not(feature = "cookies"))]
    {
        let _ = agent;
        let _ = uri;
    }
    #[cfg(feature = "cookies")]
    {
        let value = agent.jar.get_request_cookies(uri);
        if !value.is_empty() {
            let value = HeaderValue::from_str(&value)
                .map_err(|_| Error::CookieValue("Cookie value is an invalid http-header"))?;
            flow.header("cookie", value)?;
        }
    }

    #[cfg(any(feature = "gzip", feature = "brotli"))]
    {
        use once_cell::sync::Lazy;
        static ACCEPTS: Lazy<String> = Lazy::new(|| {
            let mut value = String::with_capacity(10);
            #[cfg(feature = "gzip")]
            value.push_str("gzip");
            #[cfg(all(feature = "gzip", feature = "brotli"))]
            value.push_str(", ");
            #[cfg(feature = "brotli")]
            value.push_str("br");
            value
        });
        // unwrap is ok because above ACCEPTS will produce a valid value
        let value = HeaderValue::from_str(&ACCEPTS).unwrap();
        if !has_header_accept_enc {
            flow.header("accept-encoding", value)?;
        }
    }

    if let Some(send_body_mode) = send_body_mode {
        match send_body_mode {
            BodyMode::LengthDelimited(v) => {
                let value = HeaderValue::from(v);
                flow.header("content-length", value)?;
            }
            BodyMode::Chunked => {
                let value = HeaderValue::from_static("chunked");
                flow.header("transfer-encoding", value)?;
            }
            _ => {}
        }
    }

    if !has_header_ua {
        // unwrap is ok because a user might override the agent, and if they
        // set bad values, it's not really a big problem.
        let value = HeaderValue::try_from(config.get_user_agent()).unwrap();
        flow.header("user-agent", value)?;
    }

    if !has_header_accept {
        let value = HeaderValue::from_static("*/*");
        flow.header("accept", value)?;
    }

    Ok(())
}

fn connect(
    agent: &Agent,
    config: &Config,
    uri: &Uri,
    timings: &mut CallTimings,
) -> Result<Connection, Error> {
    // If we're using a CONNECT proxy, we need to resolve that hostname.
    let maybe_connect_uri = config.connect_proxy_uri();

    let effective_uri = maybe_connect_uri.unwrap_or(uri);

    // Before resolving the URI we need to ensure it is a full URI. We
    // cannot make requests with partial uri like "/path".
    effective_uri.ensure_valid_url()?;

    let addrs = agent.resolver.resolve(
        effective_uri,
        config,
        timings.next_timeout(Timeout::Resolve),
    )?;

    timings.record_time(Timeout::Resolve);

    let details = ConnectionDetails {
        uri,
        addrs,
        resolver: &*agent.resolver,
        config,
        now: timings.now(),
        timeout: timings.next_timeout(Timeout::Connect),
    };

    let connection = agent.pool.connect(&details, config.max_idle_age.into())?;

    timings.record_time(Timeout::Connect);

    Ok(connection)
}

fn send_request(
    mut flow: Flow<SendRequest>,
    connection: &mut Connection,
    timings: &mut CallTimings,
) -> Result<SendRequestResult<()>, Error> {
    loop {
        if flow.can_proceed() {
            break;
        }

        let buffers = connection.buffers();
        let amount = flow.write(buffers.output())?;
        let timeout = timings.next_timeout(Timeout::SendRequest);
        connection.transmit_output(amount, timeout)?;
    }

    timings.record_time(Timeout::SendRequest);
    Ok(flow.proceed().unwrap())
}

fn await_100(
    mut flow: Flow<Await100>,
    connection: &mut Connection,
    timings: &mut CallTimings,
) -> Result<Await100Result<()>, Error> {
    while flow.can_keep_await_100() {
        let timeout = timings.next_timeout(Timeout::Await100);

        if timeout.after.is_zero() {
            // Stop waiting for 100-continue.a
            break;
        }

        match connection.await_input(timeout) {
            Ok(_) => {
                let input = connection.buffers().input();
                if input.is_empty() {
                    return Err(Error::disconnected());
                }

                let amount = flow.try_read_100(input)?;
                connection.consume_input(amount);
            }
            Err(Error::Timeout(_)) => {
                // If we get a timeout while waiting for input, that is not an error,
                // we progress to send the request body.
                break;
            }
            Err(e) => return Err(e),
        }
    }

    timings.record_time(Timeout::Await100);
    Ok(flow.proceed())
}

fn send_body(
    mut flow: Flow<SendBodyState>,
    body: &mut SendBody,
    connection: &mut Connection,
    timings: &mut CallTimings,
) -> Result<Flow<RecvResponse>, Error> {
    loop {
        if flow.can_proceed() {
            break;
        }

        let buffers = connection.buffers();

        let (tmp, output) = buffers.tmp_and_output();

        let input_len = tmp.len();

        let overhead = flow.calculate_output_overhead(output.len())?;
        assert!(input_len > overhead);
        let max_input = input_len - overhead;

        let output_used = if overhead == 0 {
            // overhead == 0 means we are not doing chunked transfer. The body can be written
            // directly to the output. This optimizes away a memcopy if we were to go via
            // flow.write().
            let output_used = body.read(output)?;

            // Size checking is still in the flow.
            flow.consume_direct_write(output_used)?;

            output_used
        } else {
            let tmp = &mut tmp[..max_input];
            let n = body.read(tmp)?;

            let (input_used, output_used) = flow.write(&tmp[..n], output)?;

            // Since output is "a bit" larger than the input (compensate for chunk ovexrhead),
            // the entire input we read from the body should also be shipped to the output.
            assert!(input_used == n);

            output_used
        };

        let timeout = timings.next_timeout(Timeout::SendBody);
        connection.transmit_output(output_used, timeout)?;
    }

    timings.record_time(Timeout::SendBody);
    Ok(flow.proceed().unwrap())
}

fn recv_response(
    mut flow: Flow<RecvResponse>,
    connection: &mut Connection,
    config: &Config,
    timings: &mut CallTimings,
) -> Result<(Response<()>, RecvResponseResult<()>), Error> {
    let response = loop {
        let timeout = timings.next_timeout(Timeout::RecvResponse);
        let made_progress = connection.await_input(timeout)?;

        let input = connection.buffers().input();

        let (amount, maybe_response) = flow.try_response(input)?;

        if input.len() > config.max_response_header_size {
            return Err(Error::LargeResponseHeader(
                input.len(),
                config.max_response_header_size,
            ));
        }

        connection.consume_input(amount);

        if let Some(response) = maybe_response {
            assert!(flow.can_proceed());
            break response;
        } else if !made_progress {
            return Err(Error::disconnected());
        }
    };

    timings.record_time(Timeout::RecvResponse);
    Ok((response, flow.proceed().unwrap()))
}

fn handle_redirect(mut flow: Flow<Redirect>, config: &Config) -> Result<Flow<Prepare>, Error> {
    let maybe_new_flow = flow.as_new_flow(config.redirect_auth_headers)?;
    let status = flow.status();

    if let Some(flow) = maybe_new_flow {
        info!(
            "Redirect ({}): {} {:?}",
            status,
            flow.method(),
            DebugUri(flow.uri())
        );

        Ok(flow)
    } else {
        Err(Error::RedirectFailed)
    }
}

fn cleanup(connection: Connection, must_close: bool, now: Instant) {
    if must_close {
        connection.close();
    } else {
        connection.reuse(now)
    }
}

#[derive(Default)]
pub(crate) struct BodyHandler {
    flow: Option<Flow<RecvBody>>,
    connection: Option<Connection>,
    timings: CallTimings,
    remote_closed: bool,
    redirect: Option<Flow<Redirect>>,
}

impl BodyHandler {
    fn do_read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let (Some(flow), Some(connection), timings) =
            (&mut self.flow, &mut self.connection, &mut self.timings)
        else {
            return Ok(0);
        };

        loop {
            let body_fulfilled = match flow.body_mode() {
                BodyMode::NoBody => unreachable!("must be a BodyMode for BodyHandler"),
                // These modes are fulfilled by either reaching the content-length or
                // receiving an end chunk delimiter.
                BodyMode::LengthDelimited(_) | BodyMode::Chunked => flow.can_proceed(),
                // This mode can only end once remote closes
                BodyMode::CloseDelimited => false,
            };

            if body_fulfilled {
                self.ended()?;
                return Ok(0);
            }

            let has_buffered_input = connection.buffers().can_use_input();

            // First try to use input already buffered
            if has_buffered_input {
                let input = connection.buffers().input();
                let (input_used, output_used) = flow.read(input, buf)?;
                connection.consume_input(input_used);

                if output_used > 0 {
                    return Ok(output_used);
                }

                if input_used > 0 {
                    // Body might be fulfilled now.
                    continue;
                }
            }

            if self.remote_closed {
                // we should not try to await_input again.
                self.ended()?;
                return Ok(0);
            }

            let timeout = timings.next_timeout(Timeout::RecvBody);

            let made_progress = match connection.await_input(timeout) {
                Ok(v) => v,
                Err(Error::Io(e)) => match e.kind() {
                    io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset => {
                        self.remote_closed = true;
                        true
                    }
                    _ => return Err(Error::Io(e)),
                },
                Err(e) => return Err(e),
            };

            let input = connection.buffers().input();
            let input_ended = input.is_empty();

            let (input_used, output_used) = flow.read(input, buf)?;
            connection.consume_input(input_used);

            if output_used > 0 {
                return Ok(output_used);
            } else if input_ended {
                self.ended()?;
                return Ok(0);
            } else if made_progress {
                // The await_input() made progress, but handled amount is 0. This
                // can for instance happen if we read some data, but not enough for
                // decoding any gzip.
                continue;
            } else {
                // This is an error case we don't want to see.
                return Err(Error::BodyStalled);
            }
        }
    }

    fn ended(&mut self) -> Result<(), Error> {
        self.timings.record_time(Timeout::RecvBody);

        let flow = self.flow.take().expect("ended() called with body");

        if !flow.can_proceed() {
            return Err(Error::disconnected());
        }

        let must_close_connection = match flow.proceed().unwrap() {
            RecvBodyResult::Redirect(flow) => {
                let c = flow.must_close_connection();
                self.redirect = Some(flow);
                c
            }
            RecvBodyResult::Cleanup(v) => v.must_close_connection(),
        };

        let connection = self.connection.take().expect("ended() called with body");
        cleanup(connection, must_close_connection, self.timings.now());

        Ok(())
    }

    fn consume_redirect_body(&mut self) -> Result<Flow<Redirect>, Error> {
        let mut buf = vec![0; 1024];
        loop {
            let amount = self.do_read(&mut buf)?;
            if amount == 0 {
                break;
            }
        }

        // Unwrap is OK, because we are only consuming the redirect body if
        // such a body was signalled by the remote.
        let redirect = self.redirect.take();
        Ok(redirect.expect("remote to have signaled redirect"))
    }
}

impl io::Read for BodyHandler {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.do_read(buf).map_err(|e| e.into_io())
    }
}
