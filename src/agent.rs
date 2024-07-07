use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::Duration;

use hoot::client::flow::RedirectAuthHeaders;
use http::{Request, Response, Uri};

use crate::body::RecvBody;
use crate::pool::{Connection, ConnectionPool};
use crate::resolver::{DefaultResolver, Resolver};
use crate::time::Instant;
use crate::transport::{Socket, Transport};
use crate::unit::{Event, Input, Unit};
use crate::{Body, Error};

#[derive(Debug)]
pub struct Agent {
    config: AgentConfig,
    pool: ConnectionPool,
    resolver: Box<dyn Resolver>,
}

/// Config as built by AgentBuilder and then static for the lifetime of the Agent.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Timeout for the entire call
    ///
    /// This is end-to-end, from DNS lookup to finishing reading the response body.
    /// Thus it covers all other timeouts.
    pub timeout_global: Option<Duration>,

    /// Timeout for call-by-call when following redirects
    ///
    /// This covers a single call and the timeout is reset when
    /// ureq follows a redirections.
    pub timeout_per_call: Option<Duration>,

    /// Max duration for doing the DNS lookup when establishing the connection
    ///
    /// Because most platforms do not have an async syscall for looking up
    /// a host name, setting this might force str0m to spawn a thread to handle
    /// the timeout.
    pub timeout_resolve: Option<Duration>,

    /// Max duration for establishing the connection
    ///
    /// For a TLS connection this includes opening the socket and doing the TLS handshake.
    pub timeout_connect: Option<Duration>,

    /// Max duration for sending the request, but not the request body.
    pub timeout_send_request: Option<Duration>,

    /// Max duration for awaiting a 100-continue response.
    ///
    /// Only used if there is a request body and we sent the `Expect: 100-continue`
    /// header to indicate we want the server to respond with 100.
    ///
    /// This defaults to 1 second.
    pub timeout_await_100: Option<Duration>,

    /// Max duration for sending a request body (if there is one)
    pub timeout_send_body: Option<Duration>,

    /// Max duration for receiving the response headers, but not the body
    pub timeout_recv_response: Option<Duration>,

    /// Max duration for receving the response body.
    pub timeout_recv_body: Option<Duration>,

    /// Whether to limit requests (including redirects) to https only
    ///
    /// Defaults to `false`.
    pub https_only: bool,

    /// Disable the nagle algorithm
    ///
    /// TODO(martin): more words here
    pub no_delay: bool,

    /// The max number of redirects to follow before giving up
    ///
    /// Defaults to 10
    pub max_redirects: u32,

    /// How to handle `Authorization` headers when following redirects
    ///
    /// * `Never` (the default) means the authorization header is never attached to a redirected call.
    /// * `SameHost` will keep the header when the redirect is to the same host and under https.
    pub redirect_auth_headers: RedirectAuthHeaders,

    /// Value to use for the `User-Agent` field
    ///
    /// Defaults to `ureq <version>`
    pub user_agent: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            timeout_global: None,
            timeout_per_call: None,
            timeout_resolve: None,
            timeout_connect: None,
            timeout_send_request: None,
            timeout_await_100: Some(Duration::from_secs(1)),
            timeout_send_body: None,
            timeout_recv_response: None,
            timeout_recv_body: None,
            https_only: false,
            no_delay: true,
            max_redirects: 10,
            redirect_auth_headers: RedirectAuthHeaders::Never,
            user_agent: "ureq".to_string(), // TODO(martin): add version
        }
    }
}

impl Agent {
    pub fn new(config: AgentConfig, pool: impl Transport, resolver: impl Resolver) -> Self {
        Agent {
            config,
            pool: ConnectionPool::new(pool),
            resolver: Box::new(resolver),
        }
    }

    pub(crate) fn new_default() -> Self {
        Agent::new(
            AgentConfig::default(),
            RustlConnectionPool,
            DefaultResolver::default(),
        )
    }

    // TODO(martin): Can we improve this signature? The ideal would be:
    // fn run(&self, request: Request<impl Body>) -> Result<Response<impl Body>, Error>

    // TODO(martin): One design idea is to be able to create requests in one thread, then
    // actually run them to completion in another. &mut self here makes it impossible to use
    // Agent in such a design. Is that a concern?
    pub(crate) fn run(
        &mut self,
        request: &Request<()>,
        body: Body,
    ) -> Result<Response<RecvBody>, Error> {
        let start_time = Instant::now();

        let mut unit = Unit::new(&self.config, start_time, request, body)?;

        let mut addr = None;
        let mut connection: Option<Connection> = None;
        let mut response = None;

        loop {
            // The buffer is owned by the connection. Before we have an open connection,
            // there is no buffer.
            let buffer = connection
                .as_mut()
                .map(|c| c.buffer_borrow())
                .unwrap_or(&mut []);

            match unit.poll_event(Instant::now(), buffer)? {
                Event::Reset => {
                    addr = None;
                    connection = None;
                    response = None;
                    unit.handle_input(Instant::now(), Input::Begin)?;
                }
                Event::Resolve { uri, timeout } => {
                    addr = Some(self.resolver.resolve(uri, timeout)?);
                    unit.handle_input(Instant::now(), Input::Resolved)?;
                }
                Event::OpenConnection { uri, timeout } => {
                    let addr = addr.expect("addr to be available after Event::Resolve");
                    connection = Some(self.pool.connect(uri, addr, timeout)?);
                    unit.handle_input(Instant::now(), Input::ConnectionOpen)?;
                }
                Event::Await100 { timeout } => {
                    let connection = connection.as_mut().expect("connection for AwaitInput");

                    match connection.input_await(timeout) {
                        Ok(input) => unit.handle_input(Instant::now(), Input::Input { input })?,

                        // If we get a timeout while waiting for input, that is not an error,
                        // EndAwait100 progresses the state machine to start reading a response.
                        Err(Error::Timeout(_)) => {
                            unit.handle_input(Instant::now(), Input::EndAwait100)?
                        }
                        Err(e) => return Err(e),
                    };
                }
                Event::Transmit { amount, timeout } => {
                    let connection = connection.as_mut().expect("connection for Transmit");
                    connection.buffer_transmit(amount, timeout)?;
                }
                Event::AwaitInput { timeout } => {
                    let connection = connection.as_mut().expect("connection for AwaitInput");
                    let input = connection.input_await(timeout)?;
                    unit.handle_input(Instant::now(), Input::Input { input })?;
                }
                Event::InputConsumed { amount } => {
                    let connection = connection.as_mut().expect("connection for InputConsumed");
                    connection.input_consume(amount);
                }
                Event::Response { response: r } => {
                    response = Some(r);
                }
            }
        }

        todo!()
    }
}

#[derive(Debug)]
pub struct RustlConnectionPool;

impl Transport for RustlConnectionPool {
    fn connect(
        &mut self,
        _uri: &Uri,
        addr: SocketAddr,
        timeout: Duration,
    ) -> Result<Box<dyn Socket>, Error> {
        todo!()
    }
}
