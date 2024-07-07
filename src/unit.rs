use std::collections::VecDeque;
use std::mem;
use std::time::Duration;

use hoot::client::flow::{state::*, Await100Result, RecvResponseResult, SendRequestResult};
use http::{Request, Response, Uri};

use crate::error::TimeoutReason;
use crate::time::Instant;
use crate::transport::Buffers;
use crate::{AgentConfig, Body, Error};

pub(crate) struct Unit<'c, 'a, 'b> {
    config: &'c AgentConfig,
    global_start: Instant,
    call_timings: CallTimings,
    state: State<'a>,
    body: Body<'b>,
    queued_event: VecDeque<Event<'static>>,
    redirect_count: u32,
}

type Flow<'a, State> = hoot::client::flow::Flow<'a, (), State>;

enum State<'a> {
    Begin(Flow<'a, Prepare>),
    Resolve(Flow<'a, Prepare>),
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

macro_rules! extract {
    ($e:expr, $p:path) => {
        match mem::replace($e, State::Empty) {
            $p(value) => Some(value),
            _ => None,
        }
    };
}

pub enum Event<'a> {
    Reset,
    Resolve { uri: &'a Uri, timeout: Duration },
    OpenConnection { uri: &'a Uri, timeout: Duration },
    Await100 { timeout: Duration },
    Transmit { amount: usize, timeout: Duration },
    AwaitInput { timeout: Duration, is_body: bool },
    Response { response: Response<()>, end: bool },
    ResponseBody { amount: usize },
}

pub enum Input<'a> {
    Begin,
    Resolved,
    ConnectionOpen,
    EndAwait100,
    Input { input: &'a [u8] },
}

// impl<'c, 'a, 'b> Unit<'c, 'a, 'b> {
impl<'c, 'b, 'a> Unit<'c, 'b, 'a> {
    pub fn new(
        config: &'c AgentConfig,
        global_start: Instant,
        request: &'b Request<()>,
        body: Body<'a>,
    ) -> Result<Self, Error> {
        Ok(Self {
            config,
            global_start,
            call_timings: CallTimings::default(),
            state: State::Begin(Flow::new(request)?),
            body,
            queued_event: VecDeque::new(),
            redirect_count: 0,
        })
    }

    fn global_timeout(&self) -> Instant {
        self.config
            .timeout_global
            .map(|t| self.global_start + t)
            .unwrap_or(Instant::NotHappening)
    }

    pub fn poll_event(&mut self, now: Instant, buffers: Buffers) -> Result<Event, Error> {
        let Buffers { input, output } = buffers;

        // Queued events go first.
        if let Some(queued) = self.queued_event.pop_front() {
            return Ok(queued);
        }

        let call_timeout_at = self.call_timings.next_timeout(&self.state, &self.config);
        let call_timeout = call_timeout_at.duration_since(now);

        let global_timeout_at = self.global_timeout();
        let global_timeout = global_timeout_at.duration_since(now);

        let timeout = call_timeout.min(global_timeout);

        if timeout.is_zero() {
            return Err(Error::Timeout(if global_timeout.is_zero() {
                TimeoutReason::Global
            } else {
                TimeoutReason::Call
            }));
        }

        // These outputs don't borrow from the State, but they might proceed the FSM. Hence
        // we return an Output<'static> meaning we are free the call self.maybe_change_state()
        // since self.state is not borrowed.
        let output: Option<Event<'static>> = match &mut self.state {
            State::Begin(_) => Some(Event::Reset),

            // State::Resolve (see below)
            // State::OpenConnection (see below)
            State::SendRequest(flow) => {
                let output_used = flow.write(output)?;

                Some(Event::Transmit {
                    amount: output_used,
                    timeout,
                })
            }

            State::SendBody(flow) => {
                let input_len = input.len();

                // The + 1 and floor() is to make even powers of 16 right.
                // The + 4 is for the \r\n overhead. A chunk is:
                // <digits_in_hex>\r\n
                // <chunk>\r\n
                // 0\r\n
                // \r\n
                let chunk_overhead = ((output.len() as f64).log(16.0) + 1.0).floor() as usize + 4;
                assert!(input_len > chunk_overhead);
                let max_input = input_len - chunk_overhead;

                // TODO(martin): for any body that is BodyInner::ByteSlice, it's not great to
                // go via self.body.read() since we're incurring on more memcopy than we need.
                let input = &mut input[..max_input];
                let n = self.body.read(input)?;

                let (input_used, output_used) = flow.write(&input[..n], output)?;

                // Since output is "a bit" larger than the input (compensate for chunk ovherhead),
                // the entire input we read from the body should also be shipped to the output.
                assert!(input_used == n);

                Some(Event::Transmit {
                    amount: output_used,
                    timeout,
                })
            }

            State::Await100(_) => Some(Event::Await100 { timeout }),

            State::RecvResponse(_) => Some(Event::AwaitInput {
                timeout,
                is_body: false,
            }),

            State::RecvBody(_) => Some(Event::AwaitInput {
                timeout,
                is_body: true,
            }),

            State::Redirect(flow) => {
                let maybe_new_flow = flow.as_new_flow(self.config.redirect_auth_headers)?;

                if let Some(flow) = maybe_new_flow {
                    // Start over the state
                    self.state = State::Begin(flow);

                    // Tell caller to reset state
                    Some(Event::Reset)
                } else {
                    return Err(Error::RedirectFailed);
                }
            }

            State::Cleanup(_) => todo!(),

            State::Empty => unreachable!("self.state should never be in State::Empty"),

            _ => None,
        };

        if let Some(output) = output {
            self.poll_output_maybe_proceed_state(now);
            return Ok(output);
        }

        // These Outputs borrow from the State, but they don't proceed the FSM.
        let output = match &mut self.state {
            State::Resolve(flow) => Event::Resolve {
                uri: flow.uri(),
                timeout,
            },

            State::OpenConnection(flow) => Event::OpenConnection {
                uri: flow.uri(),
                timeout,
            },

            _ => unreachable!("State must be covered in first or second match"),
        };

        Ok(output)
    }

    fn poll_output_maybe_proceed_state(&mut self, now: Instant) {
        let state = mem::replace(&mut self.state, State::Empty);

        let new_state = match state {
            // State might move on poll_output
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
                if flow.can_proceed() || self.body.is_ended() {
                    self.call_timings.time_send_body = Some(now);
                    State::RecvResponse(flow.proceed().unwrap())
                } else {
                    State::SendBody(flow)
                }
            }

            // State might move on handle_input()
            State::Begin(flow) => State::Begin(flow),
            State::Resolve(flow) => State::Resolve(flow),
            State::OpenConnection(flow) => State::OpenConnection(flow),
            State::Await100(flow) => State::Await100(flow),
            State::RecvResponse(flow) => State::RecvResponse(flow),

            // TODO(martin): decide when state moves
            State::RecvBody(flow) => State::RecvBody(flow),
            State::Redirect(flow) => State::Redirect(flow),
            State::Cleanup(flow) => State::Cleanup(flow),
            State::Empty => unreachable!("self.state should never be State::Empty"),
        };

        self.state = new_state;
    }

    pub fn handle_input(
        &mut self,
        now: Instant,
        input: Input,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        match input {
            Input::Begin => {
                let flow = extract!(&mut self.state, State::Begin)
                    .expect("Input::Begin requires State::Begin");

                self.call_timings.time_call_start = Some(now);
                self.state = State::Resolve(flow);
            }

            Input::Resolved => {
                let flow = extract!(&mut self.state, State::Resolve)
                    .expect("Input::Resolved requires State::Resolve");

                self.call_timings.time_resolve = Some(now);
                self.state = State::OpenConnection(flow)
            }

            Input::ConnectionOpen => {
                let flow = extract!(&mut self.state, State::OpenConnection)
                    .expect("Input::ConnectionOpen requires State::OpenConnection");

                self.call_timings.time_connect = Some(now);
                self.state = State::SendRequest(flow.proceed());
            }

            Input::EndAwait100 => self.end_await_100(now),

            Input::Input { input } => match &mut self.state {
                State::Await100(flow) => {
                    let input_used = flow.try_read_100(input)?;

                    // If we did indeed receive a 100-continue, we can't keep waiting for it,
                    // so the state progresses.
                    if !flow.can_keep_await_100() {
                        self.end_await_100(now);
                    }

                    return Ok(input_used);
                }

                State::RecvResponse(flow) => {
                    let (input_used, maybe_response) = flow.try_response(input)?;

                    let response = match maybe_response {
                        Some(v) => v,
                        None => return Ok(input_used),
                    };

                    let end = if response.status().is_redirection() {
                        self.redirect_count += 1;
                        // If we reached max redirections set end: true to
                        // make outer loop stop and return the body.
                        self.redirect_count < self.config.max_redirects
                    } else {
                        true
                    };

                    self.queued_event
                        .push_back(Event::Response { response, end });

                    let flow = extract!(&mut self.state, State::RecvResponse)
                        .expect("Input::Input requires State::RecvResponse");

                    let state = match flow.proceed().unwrap() {
                        RecvResponseResult::RecvBody(flow) => State::RecvBody(flow),
                        RecvResponseResult::Redirect(flow) => State::Redirect(flow),
                        RecvResponseResult::Cleanup(flow) => State::Cleanup(flow),
                    };

                    self.call_timings.time_recv_response = Some(now);
                    self.state = state;

                    return Ok(input_used);
                }

                State::RecvBody(flow) => {
                    let (input_used, output_used) = flow.read(input, output)?;

                    self.queued_event.push_back(Event::ResponseBody {
                        amount: output_used,
                    });

                    return Ok(input_used);
                }
                _ => {}
            },
        }

        Ok(0)
    }

    fn end_await_100(&mut self, now: Instant) {
        let flow = extract!(&mut self.state, State::Await100)
            .expect("Input::EndAwait100 requires State::Await100");

        self.call_timings.time_await_100 = Some(now);
        self.state = match flow.proceed() {
            Await100Result::SendBody(flow) => State::SendBody(flow),
            Await100Result::RecvResponse(flow) => State::RecvResponse(flow),
        };
    }
}

#[derive(Debug, Default)]
pub(crate) struct CallTimings {
    pub time_call_start: Option<Instant>,
    pub time_resolve: Option<Instant>,
    pub time_connect: Option<Instant>,
    pub time_send_request: Option<Instant>,
    pub time_send_body: Option<Instant>,
    pub time_await_100: Option<Instant>,
    pub time_recv_response: Option<Instant>,
    pub time_recv_body: Option<Instant>,
}

impl CallTimings {
    fn next_timeout(&self, state: &State, config: &AgentConfig) -> Instant {
        // self.time_xxx unwraps() below are OK. If the unwrap fails, we have a state
        // bug where we progressed to a certain State without setting the corresponding time.
        match state {
            State::Begin(_) => None,
            State::Resolve(_) => config
                .timeout_resolve
                .map(|t| self.time_call_start.unwrap() + t),
            State::OpenConnection(_) => config
                .timeout_connect
                .map(|t| self.time_resolve.unwrap() + t),
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
