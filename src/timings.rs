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
            Timeout::RecvResponse => &[Timeout::SendRequest, Timeout::Await100, Timeout::SendBody],
            Timeout::RecvBody => &[Timeout::RecvResponse],
            _ => &[],
        };

        prev.iter().copied()
    }

    /// All timeouts to check
    fn timeouts_to_check(&self) -> impl Iterator<Item = Timeout> {
        // Always check Global and PerCall
        once(*self).chain([Timeout::Global, Timeout::PerCall])
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
                let timeout = to_check.configured_timeout(&self.timeouts)?;
                // Global and PerCall record their starts. Other timestamps
                // record completion, which starts the next phase's budget.
                let time = match to_check {
                    Timeout::Global | Timeout::PerCall => self.time_of(to_check),
                    _ => to_check
                        .preceeding()
                        .filter_map(|previous| self.time_of(previous))
                        .max(),
                }
                .expect("timeout has no recorded start");
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

#[cfg(test)]
mod test {
    use std::sync::Mutex;
    use std::time::Duration as StdDuration;

    use super::*;

    fn with_clock(timeouts: Timeouts) -> (CallTimings, Arc<Mutex<Instant>>) {
        let clock = Arc::new(Mutex::new(Instant::now()));
        let current_time = CurrentTime(Arc::new({
            let clock = Arc::clone(&clock);
            move || *clock.lock().unwrap()
        }));
        (CallTimings::new(timeouts, current_time), clock)
    }

    #[test]
    fn response_timeout_does_not_limit_body() {
        let (mut timings, clock) = with_clock(Timeouts {
            recv_response: Some(StdDuration::from_secs(10)),
            ..Timeouts::default()
        });
        for phase in [
            Timeout::Resolve,
            Timeout::Connect,
            Timeout::SendRequest,
            Timeout::RecvResponse,
        ] {
            timings.record_time(phase);
        }
        let start = timings.now();
        *clock.lock().unwrap() = start + Duration::from_secs(20);
        assert_eq!(
            timings.next_timeout(Timeout::RecvBody),
            NextTimeout {
                after: Duration::NotHappening,
                reason: Timeout::Global
            }
        );
    }

    #[test]
    fn phase_budget_starts_after_latest_predecessor() {
        use Timeout::*;

        let cases: &[(Timeout, &[Timeout], u64)] = &[
            (Resolve, &[], 10),
            (Connect, &[Resolve], 10),
            (SendRequest, &[Resolve, Connect], 2),
            (Await100, &[Resolve, Connect, SendRequest], 3),
            (SendBody, &[Resolve, Connect, SendRequest], 20),
            (SendBody, &[Resolve, Connect, SendRequest, Await100], 20),
            (RecvResponse, &[Resolve, Connect, SendRequest], 30),
            (RecvResponse, &[Resolve, Connect, SendRequest, SendBody], 30),
            (RecvResponse, &[Resolve, Connect, SendRequest, Await100], 30),
            (
                RecvResponse,
                &[Resolve, Connect, SendRequest, Await100, SendBody],
                30,
            ),
            (RecvBody, &[Resolve, Connect, SendRequest, RecvResponse], 40),
        ];
        for &(phase, predecessors, budget) in cases {
            let (mut timings, clock) = with_clock(Timeouts {
                resolve: Some(StdDuration::from_secs(10)),
                connect: Some(StdDuration::from_secs(10)),
                send_request: Some(StdDuration::from_secs(2)),
                await_100: Some(StdDuration::from_secs(3)),
                send_body: Some(StdDuration::from_secs(20)),
                recv_response: Some(StdDuration::from_secs(30)),
                recv_body: Some(StdDuration::from_secs(40)),
                ..Timeouts::default()
            });
            let mut start = timings.now();
            for &predecessor in predecessors {
                start = start + Duration::from_secs(1);
                *clock.lock().unwrap() = start;
                timings.record_time(predecessor);
            }
            for elapsed in [0, 1, budget, budget + 1] {
                *clock.lock().unwrap() = start + Duration::from_secs(elapsed);
                assert_eq!(
                    timings.next_timeout(phase),
                    NextTimeout {
                        after: Duration::from_secs(budget.saturating_sub(elapsed)),
                        reason: phase,
                    },
                    "{phase:?} after {predecessors:?}, elapsed {elapsed}"
                );
            }
        }
    }

    #[test]
    fn redirects_reset_per_call_but_not_global_budget() {
        let (mut timings, clock) = with_clock(Timeouts {
            global: Some(StdDuration::from_secs(30)),
            per_call: Some(StdDuration::from_secs(10)),
            resolve: Some(StdDuration::from_secs(20)),
            ..Timeouts::default()
        });
        let start = timings.now();
        *clock.lock().unwrap() = start + Duration::from_secs(4);
        assert_eq!(
            timings.next_timeout(Timeout::Resolve),
            NextTimeout {
                after: Duration::from_secs(6),
                reason: Timeout::PerCall
            }
        );
        for elapsed in [6, 12, 18, 24] {
            *clock.lock().unwrap() = start + Duration::from_secs(elapsed);
            timings = timings.new_call();
            let remaining = 30 - elapsed;
            assert_eq!(
                timings.next_timeout(Timeout::Resolve),
                NextTimeout {
                    after: Duration::from_secs(remaining.min(10)),
                    reason: if remaining < 10 {
                        Timeout::Global
                    } else {
                        Timeout::PerCall
                    },
                }
            );
        }
        for elapsed in [27, 30, 31] {
            *clock.lock().unwrap() = start + Duration::from_secs(elapsed);
            assert_eq!(
                timings.next_timeout(Timeout::Global),
                NextTimeout {
                    after: Duration::from_secs(30_u64.saturating_sub(elapsed)),
                    reason: Timeout::Global,
                }
            );
        }
    }
}
