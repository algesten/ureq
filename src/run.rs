use std::convert::TryFrom;
use std::{io, mem};

use hoot::client::flow::state::{Await100, RecvBody, RecvResponse, Redirect, SendRequest};
use hoot::client::flow::state::{Prepare, SendBody as SendBodyState};
use hoot::client::flow::{Await100Result, RecvBodyResult, RecvResponseResult, SendRequestResult};
use hoot::BodyMode;
use http::uri::Scheme;
use http::{HeaderValue, Request, Response, Uri};

use crate::body::ResponseInfo;
use crate::config::CallTimings;
use crate::pool::Connection;
use crate::transport::time::{Duration, Instant};
use crate::transport::ConnectionDetails;
use crate::util::{DebugRequest, DebugResponse, DebugUri, HeaderMapExt, UriExt};
use crate::{Agent, AgentConfig, Body, Error, SendBody, TimeoutReason, Timeouts};

type Flow<T> = hoot::client::flow::Flow<(), T>;

pub(crate) fn run(
    agent: &Agent,
    request: Request<()>,
    mut body: SendBody,
) -> Result<Response<Body>, Error> {
    let mut redirect_count = 0;

    // Timeouts on the request level overrides the agent level.
    let timeouts = *request
        .extensions()
        .get::<Timeouts>()
        .unwrap_or(&agent.config.timeouts);

    let mut timings = CallTimings {
        timeouts,
        ..Default::default()
    };

    let mut flow = Flow::new(request)?;

    let (response, handler) = loop {
        timings.record_timeout(TimeoutReason::Global);

        let timeout = timings.next_timeout(TimeoutReason::Global);
        let timed_out = match timeout.after {
            Duration::Exact(v) => v.is_zero(),
            Duration::NotHappening => false,
        };
        if timed_out {
            return Err(Error::Timeout(TimeoutReason::Global));
        }

        match flow_run(agent, flow, &mut body, redirect_count, &mut timings)? {
            // Follow redirect
            FlowResult::Redirect(rflow) => {
                redirect_count += 1;

                flow = handle_redirect(rflow, &agent.config)?;
                timings = timings.new_call();
            }

            // Return response
            FlowResult::Response(response, handler) => break (response, handler),
        }
    };

    let (parts, _) = response.into_parts();

    let recv_body_mode = match &handler {
        BodyHandler::WithBody(flow, _, _) => flow.body_mode(),
        BodyHandler::WithoutBody => BodyMode::NoBody,
    };

    let info = ResponseInfo::new(&parts.headers, recv_body_mode);

    let body = Body::new(handler, info);

    let response = Response::from_parts(parts, body);

    let status = response.status();
    let is_err = status.is_client_error() || status.is_server_error();

    if agent.config.http_status_as_error && is_err {
        return Err(Error::StatusCode(status.as_u16()));
    }

    Ok(response)
}

enum FlowResult {
    Redirect(Flow<Redirect>),
    Response(Response<()>, BodyHandler),
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum BodyHandler {
    WithBody(Flow<RecvBody>, Connection, CallTimings),
    WithoutBody,
}

pub(crate) enum BodyHandlerRef<'a> {
    Shared(&'a mut BodyHandler),
    Owned(BodyHandler),
}

fn flow_run(
    agent: &Agent,
    mut flow: Flow<Prepare>,
    body: &mut SendBody,
    redirect_count: u32,
    timings: &mut CallTimings,
) -> Result<FlowResult, Error> {
    timings.record_timeout(crate::TimeoutReason::Global);

    let uri = flow.uri().clone();
    info!("{} {:?}", flow.method(), &DebugUri(flow.uri()));

    if agent.config.https_only && uri.scheme() != Some(&Scheme::HTTPS) {
        return Err(Error::AgentRequireHttpsOnly(uri.to_string()));
    }

    add_headers(&mut flow, agent, body, &uri)?;

    let mut connection = connect(agent, &uri, timings)?;

    let mut flow = flow.proceed();

    if log_enabled!(log::Level::Info) {
        let headers = flow.headers_map()?;

        let r = DebugRequest {
            method: flow.method(),
            uri: flow.uri(),
            version: flow.version(),
            headers,
        };
        info!("{:?}", r);
    }

    let flow = match send_request(flow, &mut connection, timings)? {
        SendRequestResult::Await100(flow) => match await_100(flow, &mut connection, timings)? {
            Await100Result::SendBody(flow) => send_body(flow, body, &mut connection, timings)?,
            Await100Result::RecvResponse(flow) => flow,
        },
        SendRequestResult::SendBody(flow) => send_body(flow, body, &mut connection, timings)?,
        SendRequestResult::RecvResponse(flow) => flow,
    };

    let (response, response_result) = recv_response(flow, &mut connection, &agent.config, timings)?;

    info!("{:?}", DebugResponse(&response));

    let ret = match response_result {
        RecvResponseResult::RecvBody(flow) => {
            let timings = std::mem::take(timings);
            let mut handler = BodyHandler::WithBody(flow, connection, timings);

            if response.status().is_redirection() && redirect_count < agent.config.max_redirects {
                let flow = handler.consume_redirect_body()?;

                FlowResult::Redirect(flow)
            } else {
                FlowResult::Response(response, handler)
            }
        }
        RecvResponseResult::Redirect(flow) => {
            cleanup(connection, flow.must_close_connection(), timings.now());

            if redirect_count >= agent.config.max_redirects {
                FlowResult::Response(response, BodyHandler::WithoutBody)
            } else {
                FlowResult::Redirect(flow)
            }
        }
        RecvResponseResult::Cleanup(flow) => {
            cleanup(connection, flow.must_close_connection(), timings.now());
            FlowResult::Response(response, BodyHandler::WithoutBody)
        }
    };

    Ok(ret)
}

fn add_headers(
    flow: &mut Flow<Prepare>,
    agent: &Agent,
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
    let _ = uri;
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

    if !has_header_ua && !agent.config.user_agent.is_empty() {
        // unwrap is ok because a user might override the agent, and if they
        // set bad values, it's not really a big problem.
        let value = HeaderValue::try_from(&agent.config.user_agent).unwrap();
        flow.header("user-agent", value)?;
    }

    if !has_header_accept {
        let value = HeaderValue::from_static("*/*");
        flow.header("accept", value)?;
    }

    Ok(())
}

fn connect(agent: &Agent, uri: &Uri, timings: &mut CallTimings) -> Result<Connection, Error> {
    // If we're using a CONNECT proxy, we need to resolve that hostname.
    let maybe_connect_uri = agent.config.connect_proxy_uri();

    let effective_uri = maybe_connect_uri.unwrap_or(uri);

    // Before resolving the URI we need to ensure it is a full URI. We
    // cannot make requests with partial uri like "/path".
    effective_uri.ensure_valid_url()?;

    let addrs = agent.resolver.resolve(
        effective_uri,
        &agent.config,
        timings.next_timeout(TimeoutReason::Resolver),
    )?;

    timings.record_timeout(TimeoutReason::Resolver);

    let details = ConnectionDetails {
        uri,
        addrs,
        resolver: &*agent.resolver,
        config: &agent.config,
        now: timings.now(),
        timeout: timings.next_timeout(TimeoutReason::OpenConnection),
    };

    let connection = agent.pool.connect(&details)?;

    timings.record_timeout(TimeoutReason::OpenConnection);

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
        let amount = flow.write(buffers.output_mut())?;
        let timeout = timings.next_timeout(TimeoutReason::SendRequest);
        connection.transmit_output(amount, timeout)?;
    }

    timings.record_timeout(TimeoutReason::SendRequest);
    Ok(flow.proceed().unwrap())
}

fn await_100(
    mut flow: Flow<Await100>,
    connection: &mut Connection,
    timings: &mut CallTimings,
) -> Result<Await100Result<()>, Error> {
    while flow.can_keep_await_100() {
        let timeout = timings.next_timeout(TimeoutReason::Await100);

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

    timings.record_timeout(TimeoutReason::Await100);
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

        let timeout = timings.next_timeout(TimeoutReason::SendBody);
        connection.transmit_output(output_used, timeout)?;
    }

    timings.record_timeout(TimeoutReason::SendBody);
    Ok(flow.proceed().unwrap())
}

fn recv_response(
    mut flow: Flow<RecvResponse>,
    connection: &mut Connection,
    config: &AgentConfig,
    timings: &mut CallTimings,
) -> Result<(Response<()>, RecvResponseResult<()>), Error> {
    let response = loop {
        let timeout = timings.next_timeout(TimeoutReason::RecvResponse);
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

    timings.record_timeout(TimeoutReason::RecvResponse);
    Ok((response, flow.proceed().unwrap()))
}

fn handle_redirect(mut flow: Flow<Redirect>, config: &AgentConfig) -> Result<Flow<Prepare>, Error> {
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

impl<'a> io::Read for BodyHandlerRef<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            BodyHandlerRef::Shared(v) => v.read(buf),
            BodyHandlerRef::Owned(v) => v.read(buf),
        }
    }
}

impl BodyHandler {
    fn do_read(
        &mut self,
        buf: &mut [u8],
        redirect: &mut Option<Flow<Redirect>>,
    ) -> Result<usize, Error> {
        let (flow, connection, timings) = match self {
            BodyHandler::WithoutBody => return Ok(0),
            BodyHandler::WithBody(flow, connection, timings) => (flow, connection, timings),
        };

        let mut remote_closed = false;

        loop {
            let body_fulfilled = match flow.body_mode() {
                BodyMode::NoBody => unreachable!("must be a BodyMode for BodyHandler"),
                // These modes are fulfilled by either reaching the content-length or
                // receiving an end chunk delimiter.
                BodyMode::LengthDelimited(_) | BodyMode::Chunked => flow.can_proceed(),
                // This mode can only end once remote closes
                BodyMode::CloseDelimited => remote_closed,
            };

            if body_fulfilled {
                self.ended(redirect)?;
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

            let timeout = timings.next_timeout(TimeoutReason::RecvBody);

            let made_progress = match connection.await_input(timeout) {
                Ok(v) => v,
                Err(Error::Io(e)) => match e.kind() {
                    io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset => {
                        remote_closed = true;
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
                self.ended(redirect)?;
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

    fn ended(&mut self, redirect: &mut Option<Flow<Redirect>>) -> Result<(), Error> {
        let handler = mem::replace(self, BodyHandler::WithoutBody);

        let BodyHandler::WithBody(flow, connection, mut timings) = handler else {
            unreachable!("ended() only from do_read() with body");
        };

        timings.record_timeout(TimeoutReason::RecvBody);

        if !flow.can_proceed() {
            return Err(Error::disconnected());
        }

        let must_close_connection = match flow.proceed().unwrap() {
            RecvBodyResult::Redirect(flow) => {
                let c = flow.must_close_connection();
                *redirect = Some(flow);
                c
            }
            RecvBodyResult::Cleanup(v) => v.must_close_connection(),
        };

        cleanup(connection, must_close_connection, timings.now());

        Ok(())
    }

    fn consume_redirect_body(&mut self) -> Result<Flow<Redirect>, Error> {
        let mut buf = vec![0; 1024];
        let mut redirect = None;
        loop {
            let amount = self.do_read(&mut buf, &mut redirect)?;
            if amount == 0 {
                break;
            }
        }
        // Unwrap is OK, because we are only consuming the redirect body if
        // such a body was signalled by the remote.
        Ok(redirect.expect("remote to have signaled redirect"))
    }
}

impl io::Read for BodyHandler {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.do_read(buf, &mut None).map_err(|e| e.into_io())
    }
}
