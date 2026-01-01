use std::fmt;
use std::iter::once;
use std::sync::Arc;

use crate::config::Timeouts;
use crate::transport::time::{Duration, Instant};

/// The various timeouts.
///
/// Each enum corresponds to a value in
/// [`ConfigBuilder::timeout_xxx`][crate::config::ConfigBuilder::timeout_global].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Timeout {
    /// Timeout for entire operation.
    Global,

    /// Timeout for the current call (when redirected).
    PerCall,

    /// Timeout in the resolver.
    Resolve,

    /// Timeout while opening the connection.
    Connect,

    /// Timeout while sending the request headers.
    SendRequest,

    /// Internal value never seen outside ureq (since awaiting 100 is expected
    /// to timeout).
    #[doc(hidden)]
    Await100,

    /// Timeout when sending then request body.
    SendBody,

    /// Timeout while receiving the response headers.
    RecvResponse,

    /// Timeout while receiving the response body.
    RecvBody,
}

impl Timeout {
    /// Give the immediate preceeding Timeout
    fn preceeding(&self) -> impl Iterator<Item = Timeout> {
        let prev: &[Timeout] = match self {
            Timeout::Resolve => &[Timeout::PerCall],
            Timeout::Connect => &[Timeout::Resolve],
            Timeout::SendRequest => &[Timeout::Connect],
            Timeout::Await100 => &[Timeout::SendRequest],
            Timeout::SendBody => &[Timeout::SendRequest, Timeout::Await100],
            Timeout::RecvResponse => &[Timeout::SendRequest, Timeout::SendBody],
            Timeout::RecvBody => &[Timeout::RecvResponse],
            _ => &[],
        };

        prev.iter().copied()
    }

    /// All timeouts to check
    fn timeouts_to_check(&self) -> impl Iterator<Item = Timeout> {
        // Always check Global and PerCall
        once(*self)
            .chain(self.preceeding())
            .chain([Timeout::Global, Timeout::PerCall])
    }

    /// Get the corresponding configured timeout
    fn configured_timeout(&self, timeouts: &Timeouts) -> Option<Duration> {
        match self {
            Timeout::Global => timeouts.global,
            Timeout::PerCall => timeouts.per_call,
            Timeout::Resolve => timeouts.resolve,
            Timeout::Connect => timeouts.connect,
            Timeout::SendRequest => timeouts.send_request,
            Timeout::Await100 => timeouts.await_100,
            Timeout::SendBody => timeouts.send_body,
            Timeout::RecvResponse => timeouts.recv_response,
            Timeout::RecvBody => timeouts.recv_body,
        }
        .map(Into::into)
    }
}

#[derive(Default, Debug)]
pub(crate) struct CallTimings {
    timeouts: Box<Timeouts>,
    current_time: CurrentTime,
    times: Vec<(Timeout, Instant)>,
}

impl CallTimings {
    pub(crate) fn new(timeouts: Timeouts, current_time: CurrentTime) -> Self {
        let mut times = Vec::with_capacity(8);

        let now = current_time.now();
        times.push((Timeout::Global, now));
        times.push((Timeout::PerCall, now));

        CallTimings {
            timeouts: Box::new(timeouts),
            current_time,
            times,
        }
    }

    pub(crate) fn new_call(mut self) -> CallTimings {
        self.times.truncate(1); // Global is in position 0.
        self.times.push((Timeout::PerCall, self.current_time.now()));

        CallTimings {
            timeouts: self.timeouts,
            current_time: self.current_time,
            times: self.times,
        }
    }

    pub(crate) fn current_time(&self) -> Arc<dyn Fn() -> Instant + Send + Sync + 'static> {
        self.current_time.0.clone()
    }

    pub(crate) fn now(&self) -> Instant {
        self.current_time.now()
    }

    pub(crate) fn record_time(&mut self, timeout: Timeout) {
        // Each time should only be recorded once
        assert!(
            self.time_of(timeout).is_none(),
            "{:?} recorded more than once",
            timeout
        );

        // There need to be at least one preceeding time recorded
        // since it follows a graph/call tree.
        let any_preceeding = timeout
            .preceeding()
            .filter_map(|to_check| self.time_of(to_check))
            .any(|_| true);

        assert!(any_preceeding, "{:?} has no preceeding", timeout);

        // Record the time
        self.times.push((timeout, self.current_time.now()));
    }

    fn time_of(&self, timeout: Timeout) -> Option<Instant> {
        self.times.iter().find(|x| x.0 == timeout).map(|x| x.1)
    }

    pub(crate) fn next_timeout(&self, timeout: Timeout) -> NextTimeout {
        let now = self.now();

        let (reason, at) = timeout
            .timeouts_to_check()
            .filter_map(|to_check| {
                let time = if to_check == timeout {
                    now
                } else {
                    self.time_of(to_check)?
                };
                let timeout = to_check.configured_timeout(&self.timeouts)?;
                Some((to_check, time + timeout))
            })
            .min_by(|a, b| a.1.cmp(&b.1))
            .unwrap_or((Timeout::Global, Instant::NotHappening));

        let after = at.duration_since(now);

        NextTimeout { after, reason }
    }
}

#[derive(Clone)]
pub(crate) struct CurrentTime(Arc<dyn Fn() -> Instant + Send + Sync + 'static>);

impl CurrentTime {
    pub(crate) fn now(&self) -> Instant {
        self.0()
    }
}

/// A pair of [`Duration`] and [`Timeout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextTimeout {
    /// Duration until next timeout.
    pub after: Duration,
    /// The name of the next timeout.s
    pub reason: Timeout,
}

impl NextTimeout {
    /// Returns the duration of the timeout if the timeout must happen, but avoid instant timeouts
    ///
    /// If the timeout must happen but is zero, returns 1 second
    pub fn not_zero(&self) -> Option<Duration> {
        if self.after.is_not_happening() {
            None
        } else if self.after.is_zero() {
            Some(Duration::from_secs(1))
        } else {
            Some(self.after)
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
            Timeout::PerCall => "per call",
            Timeout::Resolve => "resolve",
            Timeout::Connect => "connect",
            Timeout::SendRequest => "send request",
            Timeout::SendBody => "send body",
            Timeout::Await100 => "await 100",
            Timeout::RecvResponse => "receive response",
            Timeout::RecvBody => "receive body",
        };
        write!(f, "{}", r)
    }
}
