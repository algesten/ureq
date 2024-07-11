use core::fmt;
use std::collections::VecDeque;
use std::mem;
use std::sync::Arc;

use hoot::client::flow::{
    state::*, Await100Result, RecvBodyResult, RecvResponseResult, SendRequestResult,
};
use http::{Request, Response, Uri};

use crate::error::TimeoutReason;
use crate::time::{Duration, Instant};
use crate::transport::Buffers;
use crate::util::DebugResponse;
use crate::{AgentConfig, Body, Error};

pub(crate) struct Unit<B> {
    config: Arc<AgentConfig>,
    global_start: Instant,
    call_timings: CallTimings,
    state: State,
    body: B,
    queued_event: VecDeque<Event<'static>>,
    redirect_count: u32,
    prev_state: &'static str,
}

type Flow<State> = hoot::client::flow::Flow<(), State>;

enum State {
    Begin(Flow<Prepare>),
    Resolve(Flow<Prepare>),
    OpenConnection(Flow<Prepare>),
    SendRequest(Flow<SendRequest>),
    SendBody(Flow<SendBody>),
    Await100(Flow<Await100>),
    RecvResponse(Flow<RecvResponse>),
    RecvBody(Flow<RecvBody>),
    Redirect(Flow<Redirect>),
    Cleanup(Flow<Cleanup>),
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
    Reset { must_close: bool },
    Resolve { uri: &'a Uri, timeout: Duration },
    OpenConnection { uri: &'a Uri, timeout: Duration },
    Await100 { timeout: Duration },
    Transmit { amount: usize, timeout: Duration },
    AwaitInput { timeout: Duration },
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

impl<'b> Unit<Body<'b>> {
    pub fn new(
        config: Arc<AgentConfig>,
        global_start: Instant,
        request: Request<()>,
        body: Body<'b>,
    ) -> Result<Self, Error> {
        Ok(Self {
            config,
            global_start,
            call_timings: CallTimings::default(),
            state: State::Begin(Flow::new(request)?),
            body,
            queued_event: VecDeque::new(),
            redirect_count: 0,
            prev_state: "",
        })
    }

    pub fn poll_event(&mut self, now: Instant, buffers: &mut dyn Buffers) -> Result<Event, Error> {
        let event = self.do_poll_event(now, buffers)?;
        trace!("poll_event: {:?}", event);
        Ok(event)
    }

    fn do_poll_event(&mut self, now: Instant, buffers: &mut dyn Buffers) -> Result<Event, Error> {
        // Queued events go first.
        if let Some(queued) = self.queued_event.pop_front() {
            return Ok(queued);
        }

        let timeout = self.next_timeout(now)?;

        // Events that do not borrow any state, but might proceed the FSM
        let maybe_event = self.poll_event_static(buffers, timeout)?;

        if let Some(event) = maybe_event {
            self.poll_event_maybe_proceed_state(now);
            return Ok(event);
        }

        // Events that borrow the state and don't proceed the FSM.
        self.poll_event_borrow(timeout)
    }

    // These events don't borrow from the State, but they might proceed the FSM. Hence
    // we return an Event<'static> meaning we are free the call self.poll_event_maybe_proceed_state()
    // since self.state is not borrowed.
    fn poll_event_static(
        &mut self,
        buffers: &mut dyn Buffers,
        timeout: Duration,
    ) -> Result<Option<Event<'static>>, Error> {
        Ok(match &mut self.state {
            State::Begin(flow) => {
                info!("{} {}", flow.method(), flow.uri());
                Some(Event::Reset { must_close: false })
            }

            // State::Resolve (see below)
            // State::OpenConnection (see below)
            State::SendRequest(flow) => Some(send_request(flow, buffers.output_mut(), timeout)?),

            State::SendBody(flow) => Some(send_body(flow, buffers, timeout, &mut self.body)?),

            State::Await100(_) => Some(Event::Await100 { timeout }),

            State::RecvResponse(_) => Some(Event::AwaitInput { timeout }),

            State::RecvBody(_) => Some(Event::AwaitInput { timeout }),

            State::Redirect(flow) => {
                // Whether the previous connection must be closed.
                let must_close = flow.must_close_connection();

                let maybe_new_flow = flow.as_new_flow(self.config.redirect_auth_headers)?;
                let status = flow.status();

                if let Some(flow) = maybe_new_flow {
                    info!("Redirect ({}): {} {}", status, flow.method(), flow.uri());

                    // Start over the state
                    self.set_state(State::Begin(flow));

                    // Tell caller to reset state
                    Some(Event::Reset { must_close })
                } else {
                    return Err(Error::RedirectFailed);
                }
            }

            State::Cleanup(flow) => Some(Event::Reset {
                must_close: flow.must_close_connection(),
            }),

            State::Empty => unreachable!("self.state should never be in State::Empty"),

            _ => None,
        })
    }

    fn poll_event_maybe_proceed_state(&mut self, now: Instant) {
        let state = mem::replace(&mut self.state, State::Empty);

        let new_state = match state {
            // State moves on poll_output
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

            // Special handling above.
            State::Redirect(flow) => State::Redirect(flow),

            // State moves on handle_input()
            State::Begin(flow) => State::Begin(flow),
            State::Resolve(flow) => State::Resolve(flow),
            State::OpenConnection(flow) => State::OpenConnection(flow),
            State::Await100(flow) => State::Await100(flow),
            State::RecvResponse(flow) => State::RecvResponse(flow),
            State::RecvBody(flow) => State::RecvBody(flow),

            State::Cleanup(flow) => State::Cleanup(flow),

            State::Empty => unreachable!("self.state should never be State::Empty"),
        };

        self.set_state(new_state);
    }

    // These events borrow from the State, but they don't proceed the FSM.
    fn poll_event_borrow(&self, timeout: Duration) -> Result<Event, Error> {
        let event = match &self.state {
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

        Ok(event)
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
                self.set_state(State::Resolve(flow));
            }

            Input::Resolved => {
                let flow = extract!(&mut self.state, State::Resolve)
                    .expect("Input::Resolved requires State::Resolve");

                self.call_timings.time_resolve = Some(now);
                self.set_state(State::OpenConnection(flow));
            }

            Input::ConnectionOpen => {
                let flow = extract!(&mut self.state, State::OpenConnection)
                    .expect("Input::ConnectionOpen requires State::OpenConnection");

                self.call_timings.time_connect = Some(now);
                self.set_state(State::SendRequest(flow.proceed()));
            }

            Input::EndAwait100 => self.end_await_100(now),

            Input::Input { input } => match &mut self.state {
                State::Await100(flow) => {
                    if input.is_empty() {
                        return Err(Error::disconnected());
                    }

                    let input_used = flow.try_read_100(input)?;

                    // If we did indeed receive a 100-continue, we can't keep waiting for it,
                    // so the state progresses.
                    if !flow.can_keep_await_100() {
                        self.end_await_100(now);
                    }

                    return Ok(input_used);
                }

                State::RecvResponse(flow) => {
                    if input.is_empty() {
                        return Err(Error::disconnected());
                    }

                    let (input_used, maybe_response) = flow.try_response(input)?;

                    let response = match maybe_response {
                        Some(v) => v,
                        None => return Ok(input_used),
                    };

                    let end = if response.status().is_redirection() {
                        self.redirect_count += 1;
                        // If we reached max redirections set end: true to
                        // make outer loop stop and return the body.
                        self.redirect_count >= self.config.max_redirects
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
                    self.set_state(state);

                    return Ok(input_used);
                }

                State::RecvBody(_) => return self.handle_input_recv_body(now, input, output),

                _ => {}
            },
        }

        Ok(0)
    }

    fn end_await_100(&mut self, now: Instant) {
        let flow = extract!(&mut self.state, State::Await100)
            .expect("Input::EndAwait100 requires State::Await100");

        self.call_timings.time_await_100 = Some(now);
        self.set_state(match flow.proceed() {
            Await100Result::SendBody(flow) => State::SendBody(flow),
            Await100Result::RecvResponse(flow) => State::RecvResponse(flow),
        });
    }

    pub fn release_body(self) -> Unit<()> {
        Unit {
            config: self.config,
            global_start: self.global_start,
            call_timings: self.call_timings,
            state: self.state,
            body: (),
            queued_event: self.queued_event,
            redirect_count: self.redirect_count,
            prev_state: self.prev_state,
        }
    }
}

// Unit<()> is for receiving the body. We have let go of the input body.
impl Unit<()> {
    pub fn poll_event(&mut self, now: Instant) -> Result<Event, Error> {
        let event = self.do_poll_event(now)?;
        trace!("poll_event (recv): {:?}", event);
        Ok(event)
    }

    fn do_poll_event(&mut self, now: Instant) -> Result<Event, Error> {
        // Queued events go first.
        if let Some(queued) = self.queued_event.pop_front() {
            return Ok(queued);
        }

        let timeout = self.next_timeout(now)?;

        match &self.state {
            State::RecvBody(_) => Ok(Event::AwaitInput { timeout }),
            State::Cleanup(flow) => Ok(Event::Reset {
                must_close: flow.must_close_connection(),
            }),
            State::Redirect(flow) => Ok(Event::Reset {
                must_close: flow.must_close_connection(),
            }),
            _ => unreachable!(),
        }
    }

    pub fn handle_input(
        &mut self,
        now: Instant,
        input: Input,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        match input {
            Input::Input { input } => self.handle_input_recv_body(now, input, output),
            _ => unreachable!(),
        }
    }
}

impl<B> Unit<B> {
    fn set_state(&mut self, state: State) {
        let new_name = state.name();
        if new_name != self.prev_state {
            if self.prev_state != "" {
                trace!("{} -> {}", self.prev_state, new_name);
            } else {
                trace!("Start state: {}", new_name);
            }
            self.prev_state = new_name;
        }
        self.state = state
    }

    fn global_timeout(&self) -> Instant {
        self.config
            .timeout_global
            .map(|t| self.global_start + t)
            .unwrap_or(Instant::NotHappening)
    }

    fn next_timeout(&mut self, now: Instant) -> Result<Duration, Error> {
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

        Ok(timeout)
    }

    fn handle_input_recv_body(
        &mut self,
        now: Instant,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<usize, Error> {
        let flow = match &mut self.state {
            State::RecvBody(v) => v,
            _ => unreachable!(),
        };

        let (input_used, output_used) = flow.read(input, output)?;

        self.queued_event.push_back(Event::ResponseBody {
            amount: output_used,
        });

        if flow.can_proceed() {
            let flow = extract!(&mut self.state, State::RecvBody)
                .expect("Input::Input requires State::RecvBody");

            let state = match flow.proceed().unwrap() {
                RecvBodyResult::Redirect(flow) => State::Redirect(flow),
                RecvBodyResult::Cleanup(flow) => State::Cleanup(flow),
            };

            self.call_timings.time_recv_body = Some(now);
            self.set_state(state);
        }

        return Ok(input_used);
    }
}

fn send_request(
    flow: &mut Flow<SendRequest>,
    output: &mut [u8],
    timeout: Duration,
) -> Result<Event<'static>, Error> {
    let output_used = flow.write(output)?;

    Ok(Event::Transmit {
        amount: output_used,
        timeout,
    })
}

fn send_body(
    flow: &mut Flow<SendBody>,
    buffers: &mut dyn Buffers,
    timeout: Duration,
    body: &mut Body,
) -> Result<Event<'static>, Error> {
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

        // Since output is "a bit" larger than the input (compensate for chunk ovherhead),
        // the entire input we read from the body should also be shipped to the output.
        assert!(input_used == n);

        output_used
    };

    Ok(Event::Transmit {
        amount: output_used,
        timeout,
    })
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

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::Begin(_) => "Begin",
            State::Resolve(_) => "Resolve",
            State::OpenConnection(_) => "OpenConnection",
            State::SendRequest(_) => "SendRequest",
            State::SendBody(_) => "SendBody",
            State::Await100(_) => "Await100",
            State::RecvResponse(_) => "RecvResponse",
            State::RecvBody(_) => "RecvBody",
            State::Redirect(_) => "Redirect",
            State::Cleanup(_) => "Cleanup",
            State::Empty => "Empty (wrong!)",
        }
    }
}

impl fmt::Debug for Event<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reset { must_close } => f
                .debug_struct("Reset")
                .field("must_close", must_close)
                .finish(),
            Self::Resolve { uri, timeout } => f
                .debug_struct("Resolve")
                .field("uri", uri)
                .field("timeout", timeout)
                .finish(),
            Self::OpenConnection { uri, timeout } => f
                .debug_struct("OpenConnection")
                .field("uri", uri)
                .field("timeout", timeout)
                .finish(),
            Self::Await100 { timeout } => f
                .debug_struct("Await100")
                .field("timeout", timeout)
                .finish(),
            Self::Transmit { amount, timeout } => f
                .debug_struct("Transmit")
                .field("amount", amount)
                .field("timeout", timeout)
                .finish(),
            Self::AwaitInput { timeout } => f
                .debug_struct("AwaitInput")
                .field("timeout", timeout)
                .finish(),
            Self::Response { response, end } => f
                .debug_struct("Response")
                .field("response", &DebugResponse(&response))
                .field("end", end)
                .finish(),
            Self::ResponseBody { amount } => f
                .debug_struct("ResponseBody")
                .field("amount", amount)
                .finish(),
        }
    }
}
