use std::fmt;
use std::sync::Arc;

use crate::transport::time::{Duration, Instant};
use crate::Timeouts;

/// The various timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Timeout {
    /// Timeout for entire call.
    Global,

    /// Timeout in the resolver.
    Resolve,

    /// Timeout while opening the connection.
    Connect,

    /// Timeout while sending the request headers.
    SendRequest,

    /// Timeout when sending then request body.
    SendBody,

    /// Internal value never seen outside ureq (since awaiting 100 is expected
    /// to timeout).
    #[doc(hidden)]
    Await100,

    /// Timeout while receiving the response headers.
    RecvResponse,

    /// Timeout while receiving the response body.
    RecvBody,
}

#[derive(Debug, Default)]
pub(crate) struct CallTimings {
    pub timeouts: Timeouts,
    pub current_time: CurrentTime,

    pub time_global_start: Option<Instant>,
    pub time_call_start: Option<Instant>,
    pub time_resolve: Option<Instant>,
    pub time_connect: Option<Instant>,
    pub time_send_request: Option<Instant>,
    pub time_send_body: Option<Instant>,
    pub time_await_100: Option<Instant>,
    pub time_recv_response: Option<Instant>,
    pub time_recv_body: Option<Instant>,
}

#[derive(Clone)]
pub(crate) struct CurrentTime(Arc<dyn Fn() -> Instant + Send + Sync + 'static>);

impl CurrentTime {
    pub(crate) fn now(&self) -> Instant {
        self.0()
    }
}

/// A pair of [`Duration`] and [`TimeoutReason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextTimeout {
    /// Duration until next timeout.
    pub after: Duration,
    /// The name of the next timeout.s
    pub reason: Timeout,
}

impl NextTimeout {
    pub(crate) fn not_zero(&self) -> Option<Duration> {
        if self.after.is_not_happening() {
            None
        } else if self.after.is_zero() {
            Some(Duration::from_secs(1))
        } else {
            Some(self.after)
        }
    }
}

impl CallTimings {
    pub(crate) fn now(&self) -> Instant {
        self.current_time.now()
    }

    pub(crate) fn record_timeout(&mut self, reason: Timeout) {
        match reason {
            Timeout::Global => {
                let now = self.now();
                if self.time_global_start.is_none() {
                    self.time_global_start = Some(now);
                }
                self.time_call_start = Some(now);
            }
            Timeout::Resolve => {
                self.time_resolve = Some(self.now());
            }
            Timeout::Connect => {
                self.time_connect = Some(self.now());
            }
            Timeout::SendRequest => {
                self.time_send_request = Some(self.now());
            }
            Timeout::SendBody => {
                self.time_send_body = Some(self.now());
            }
            Timeout::Await100 => {
                self.time_await_100 = Some(self.now());
            }
            Timeout::RecvResponse => {
                self.time_recv_response = Some(self.now());
            }
            Timeout::RecvBody => {
                self.time_recv_body = Some(self.now());
            }
        }
    }

    pub(crate) fn next_timeout(&self, reason: Timeout) -> NextTimeout {
        // self.time_xxx unwraps() below are OK. If the unwrap fails, we have a state
        // bug where we progressed to a certain state without setting the corresponding time.
        let timeouts = &self.timeouts;

        let expire_at = match reason {
            Timeout::Global => timeouts
                .global
                .map(|t| self.time_global_start.unwrap() + t.into()),
            Timeout::Resolve => timeouts
                .resolve
                .map(|t| self.time_call_start.unwrap() + t.into()),
            Timeout::Connect => timeouts
                .connect
                .map(|t| self.time_resolve.unwrap() + t.into()),
            Timeout::SendRequest => timeouts
                .send_request
                .map(|t| self.time_connect.unwrap() + t.into()),
            Timeout::SendBody => timeouts
                .send_body
                .map(|t| self.time_send_request.unwrap() + t.into()),
            Timeout::Await100 => timeouts
                .await_100
                .map(|t| self.time_send_request.unwrap() + t.into()),
            Timeout::RecvResponse => timeouts.recv_response.map(|t| {
                // The fallback order is important. See state diagram in hoot.
                self.time_send_body
                    .or(self.time_await_100)
                    .or(self.time_send_request)
                    .unwrap()
                    + t.into()
            }),
            Timeout::RecvBody => timeouts
                .recv_body
                .map(|t| self.time_recv_response.unwrap() + t.into()),
        }
        .unwrap_or(Instant::NotHappening);

        let global_at = self.global_timeout();

        let (at, reason) = if global_at < expire_at {
            (global_at, Timeout::Global)
        } else {
            (expire_at, reason)
        };

        let after = at.duration_since(self.now());

        NextTimeout { after, reason }
    }

    fn global_timeout(&self) -> Instant {
        let global_start = self.time_global_start.unwrap();
        let call_start = self.time_call_start.unwrap();

        let global_at = global_start
            + self
                .timeouts
                .global
                .map(|t| t.into())
                .unwrap_or(crate::transport::time::Duration::NotHappening);

        let call_at = call_start
            + self
                .timeouts
                .per_call
                .map(|t| t.into())
                .unwrap_or(crate::transport::time::Duration::NotHappening);

        global_at.min(call_at)
    }

    pub(crate) fn new_call(self) -> CallTimings {
        CallTimings {
            timeouts: self.timeouts,
            time_global_start: self.time_global_start,
            current_time: self.current_time,
            ..Default::default()
        }
    }
}

impl fmt::Debug for CurrentTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CurrentTime").finish()
    }
}

impl Default for CurrentTime {
    fn default() -> Self {
        Self(Arc::new(Instant::now))
    }
}

impl fmt::Display for Timeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = match self {
            Timeout::Global => "global",
            Timeout::Resolve => "resolver",
            Timeout::Connect => "open connection",
            Timeout::SendRequest => "send request",
            Timeout::SendBody => "send body",
            Timeout::Await100 => "await 100",
            Timeout::RecvResponse => "receive response",
            Timeout::RecvBody => "receive body",
        };
        write!(f, "{}", r)
    }
}
