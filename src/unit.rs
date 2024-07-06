use std::mem;
use std::net::SocketAddr;
use std::sync::Arc;

use hoot::client::flow::{
    state::*, Await100Result, RecvBodyResult, RecvResponseResult, SendRequestResult,
};
use http::{Request, Uri};

use crate::time::Instant;
use crate::{AgentConfig, Body, Error};

pub(crate) struct Unit<'a, 'b> {
    config: Arc<AgentConfig>,
    time_start: Instant,
    call_timings: CallTimings,
    state: State<'a>,
    body: Body<'b>,
    addr: Option<SocketAddr>,
}

type Flow<'a, State> = hoot::client::flow::Flow<'a, (), State>;

enum State<'a> {
    Begin(Flow<'a, Prepare>),
    DnsLookup(Flow<'a, Prepare>),
    OpenConnection(Flow<'a, Prepare>),
    SendRequest(Flow<'a, SendRequest>),
    SendBody(Flow<'a, SendBody>),
    Await100(Flow<'a, Await100>),
    RecvResponse(Flow<'a, RecvResponse>),
    RecvBody(Flow<'a, RecvBody>),
    Redirect(Flow<'a, Redirect>),
    Cleanup(Flow<'a, Cleanup>),
    Empty,
}

pub enum Output<'a> {
    Reset,
    DnsLookup {
        uri: &'a Uri,
        timeout: Instant,
    },
    OpenConnection {
        uri: &'a Uri,
        addr: SocketAddr,
        timeout: Instant,
    },
    Await100 {
        timeout: Instant,
    },
    Transmit {
        amount: usize,
        timeout: Instant,
    },
    AwaitInput {
        timeout: Instant,
    },
}

pub enum Input<'a> {
    Begin,
    SocketAddr(SocketAddr),
    ConnectionOpen,
    EndAwait100,
    Input { data: &'a [u8] },
}

impl<'a, 'b> Unit<'a, 'b> {
    pub fn new(
        config: Arc<AgentConfig>,
        time_start: Instant,
        request: &'a Request<()>,
        body: Body<'b>,
    ) -> Result<Self, Error> {
        Ok(Self {
            config,
            time_start,
            call_timings: CallTimings::default(),
            state: State::Begin(Flow::new(request)?),
            body,
            addr: None,
        })
    }

    pub fn poll_output(
        &'a mut self,
        now: Instant,
        transmit_buffer: &mut [u8],
    ) -> Result<Output, Error> {
        let timeout = self.call_timings.next_timeout(&self.state, &self.config);

        // These outputs don't borrow from the State, but they might proceed the FSM. Hence
        // we return an Output<'static> meaning we are free the call self.maybe_change_state()
        // since self.state is not borrowed.
        let output: Option<Output<'static>> = match &mut self.state {
            State::Begin(_) => Some(Output::Reset),
            // State::DnsLookup (see below)
            // State::OpenConnection (see below)
            State::SendRequest(flow) => {
                let output_used = flow.write(transmit_buffer)?;

                Some(Output::Transmit {
                    amount: output_used,
                    timeout,
                })
            }
            State::SendBody(flow) => {
                // let (input_used, output_used) = flow.write(input, transmit_buffer)?;

                // Some(Output::Transmit {
                //     amount: output_used,
                //     timeout,
                // })
                todo!()
            }
            State::Await100(_) => Some(Output::Await100 { timeout }),
            State::RecvResponse(_) => Some(Output::AwaitInput { timeout }),
            State::RecvBody(_) => Some(Output::AwaitInput { timeout }),
            State::Redirect(_) => todo!(),
            State::Cleanup(_) => todo!(),
            State::Empty => unreachable!("self.state should never be in State::Empty"),
            _ => None,
        };

        if let Some(output) = output {
            self.poll_output_maybe_proceed_state(now);
            return Ok(output);
        }

        // These Outputs borrow from the State, but they don't proceed the FSM.
        Ok(match &mut self.state {
            State::DnsLookup(flow) => Output::DnsLookup {
                uri: flow.uri(),
                timeout,
            },
            State::OpenConnection(flow) => Output::OpenConnection {
                uri: flow.uri(),
                addr: self.addr.unwrap(),
                timeout,
            },
            _ => unreachable!("State must be covered in first or second match"),
        })
    }

    fn poll_output_maybe_proceed_state(&mut self, now: Instant) {
        let state = mem::replace(&mut self.state, State::Empty);

        let new_state = match state {
            State::Begin(flow) => State::Begin(flow),
            State::DnsLookup(flow) => State::DnsLookup(flow),
            State::OpenConnection(flow) => State::OpenConnection(flow),
            State::SendRequest(flow) => {
                if flow.can_proceed() {
                    self.call_timings.time_send_request = Some(now);
                    match flow.proceed().unwrap() {
                        SendRequestResult::Await100(flow) => State::Await100(flow),
                        SendRequestResult::SendBody(flow) => State::SendBody(flow),
                        SendRequestResult::RecvResponse(flow) => State::RecvResponse(flow),
                    }
                } else {
                    State::SendRequest(flow)
                }
            }
            State::SendBody(flow) => {
                if flow.can_proceed() {
                    self.call_timings.time_send_body = Some(now);
                    State::RecvResponse(flow.proceed().unwrap())
                } else {
                    State::SendBody(flow)
                }
            }
            State::Await100(flow) => State::Await100(flow),
            State::RecvResponse(flow) => {
                if flow.can_proceed() {
                    self.call_timings.time_recv_response = Some(now);
                    match flow.proceed().unwrap() {
                        RecvResponseResult::RecvBody(flow) => State::RecvBody(flow),
                        RecvResponseResult::Redirect(flow) => State::Redirect(flow),
                        RecvResponseResult::Cleanup(flow) => State::Cleanup(flow),
                    }
                } else {
                    State::RecvResponse(flow)
                }
            }
            State::RecvBody(flow) => {
                if flow.can_proceed() {
                    self.call_timings.time_recv_body = Some(now);
                    match flow.proceed().unwrap() {
                        RecvBodyResult::Redirect(flow) => State::Redirect(flow),
                        RecvBodyResult::Cleanup(flow) => State::Cleanup(flow),
                    }
                } else {
                    State::RecvBody(flow)
                }
            }
            State::Redirect(flow) => State::Redirect(flow),
            State::Cleanup(flow) => State::Cleanup(flow),
            State::Empty => unreachable!("self.state should never be State::Empty"),
        };

        self.state = new_state;
    }

    pub fn handle_input(&mut self, now: Instant, input: Input) {
        match input {
            Input::Begin => {
                let state = mem::replace(&mut self.state, State::Empty);
                let flow = match state {
                    State::Begin(v) => v,
                    _ => unreachable!("Input::Begin requires State::Begin"),
                };

                self.call_timings.time_call_start = Some(now);
                self.state = State::DnsLookup(flow);
            }
            Input::SocketAddr(addr) => {
                let state = mem::replace(&mut self.state, State::Empty);
                let flow = match state {
                    State::DnsLookup(v) => v,
                    _ => unreachable!("Input::SocketAddr requires State::DnsLookup"),
                };

                self.call_timings.time_dns_lookup = Some(now);
                self.addr = Some(addr);
                self.state = State::OpenConnection(flow)
            }
            Input::ConnectionOpen => {
                let state = mem::replace(&mut self.state, State::Empty);
                let flow = match state {
                    State::OpenConnection(v) => v,
                    _ => unreachable!("Input::SocketAddr requires State::DnsLookup"),
                };

                self.call_timings.time_connect = Some(now);
                self.state = State::SendRequest(flow.proceed());
            }
            Input::EndAwait100 => {
                let state = mem::replace(&mut self.state, State::Empty);
                let flow = match state {
                    State::Await100(v) => v,
                    _ => unreachable!("Input::SocketAddr requires State::DnsLookup"),
                };

                self.call_timings.time_await_100 = Some(now);
                self.state = match flow.proceed() {
                    Await100Result::SendBody(flow) => State::SendBody(flow),
                    Await100Result::RecvResponse(flow) => State::RecvResponse(flow),
                };
            }
            Input::Input { data } => todo!(),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct CallTimings {
    pub time_call_start: Option<Instant>,
    pub time_dns_lookup: Option<Instant>,
    pub time_connect: Option<Instant>,
    pub time_send_request: Option<Instant>,
    pub time_send_body: Option<Instant>,
    pub time_await_100: Option<Instant>,
    pub time_recv_response: Option<Instant>,
    pub time_recv_body: Option<Instant>,
}

impl CallTimings {
    fn next_timeout<'a>(&self, state: &State, config: &AgentConfig) -> Instant {
        // self.time_xxx unwraps() below are OK. If the unwrap fails, we have a state
        // bug where we progressed to a certain State without setting the corresponding time.
        match state {
            State::Begin(_) => None,
            State::DnsLookup(_) => config
                .timeout_dns_lookup
                .map(|t| self.time_call_start.unwrap() + t),
            State::OpenConnection(_) => config
                .timeout_connect
                .map(|t| self.time_dns_lookup.unwrap() + t),
            State::SendRequest(_) => config
                .timeout_send_request
                .map(|t| self.time_connect.unwrap() + t),
            State::SendBody(_) => config
                .timeout_send_body
                .map(|t| self.time_send_request.unwrap() + t),
            State::Await100(_) => config
                .timeout_await_100
                .map(|t| self.time_send_request.unwrap() + t),
            State::RecvResponse(_) => config.timeout_recv_response.map(|t| {
                // The fallback order is important. See state diagram in hoot.
                self.time_send_body
                    .or(self.time_await_100)
                    .or(self.time_send_request)
                    .unwrap()
                    + t
            }),
            State::RecvBody(_) => config
                .timeout_recv_body
                .map(|t| self.time_recv_response.unwrap() + t),
            State::Redirect(_) => None,
            State::Cleanup(_) => None,
            State::Empty => unreachable!("next_timeout should never be called for State::Empty"),
        }
        .unwrap_or(Instant::NotHappening)
    }
}
