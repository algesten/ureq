//! Internal time wrappers

use std::cmp::Ordering;
use std::ops::{Add, Deref, Div};
use std::time;

/// Wrapper for [`std::time::Instant`] that provides additional time points in the past or future
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instant {
    /// A time in the past that already happened.
    AlreadyHappened,
    /// An exact instant.
    Exact(time::Instant),
    /// A time in the future that will never happen.
    NotHappening,
}

/// Wrapper for [`std::time::Duration`] that provides a duration to a distant future
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duration {
    /// An exact duration.
    Exact(time::Duration),
    /// A duration so long it will never happen.
    NotHappening,
}

impl Duration {
    const ZERO: Duration = Duration::Exact(time::Duration::ZERO);

    /// Creates a duration from seconds.
    pub const fn from_secs(secs: u64) -> Duration {
        Duration::Exact(time::Duration::from_secs(secs))
    }

    /// Creates a duration from milliseconds.
    pub const fn from_millis(millis: u64) -> Duration {
        Duration::Exact(time::Duration::from_millis(millis))
    }

    /// Tells if this duration will ever happen.
    pub fn is_not_happening(&self) -> bool {
        *self == Duration::NotHappening
    }
}

const NOT_HAPPENING: time::Duration = time::Duration::from_secs(u64::MAX);

impl Deref for Duration {
    type Target = time::Duration;

    fn deref(&self) -> &Self::Target {
        match self {
            Duration::Exact(v) => v,
            Duration::NotHappening => &NOT_HAPPENING,
        }
    }
}

impl Instant {
    /// Current time.
    pub fn now() -> Self {
        Instant::Exact(time::Instant::now())
    }

    pub(crate) fn duration_since(&self, earlier: Instant) -> Duration {
        match (self, earlier) {
            (Instant::AlreadyHappened, Instant::AlreadyHappened) => Duration::ZERO,
            (Instant::AlreadyHappened, Instant::Exact(_)) => Duration::ZERO,
            (Instant::AlreadyHappened, Instant::NotHappening) => Duration::ZERO,
            (Instant::Exact(_), Instant::NotHappening) => Duration::ZERO,
            (Instant::Exact(v1), Instant::Exact(v2)) => {
                Duration::Exact(v1.saturating_duration_since(v2))
            }
            (Instant::Exact(_), Instant::AlreadyHappened) => Duration::NotHappening,
            (Instant::NotHappening, Instant::AlreadyHappened) => Duration::NotHappening,
            (Instant::NotHappening, Instant::Exact(_)) => Duration::NotHappening,
            (Instant::NotHappening, Instant::NotHappening) => Duration::NotHappening,
        }
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, rhs: Duration) -> Self::Output {
        match (self, rhs) {
            (Instant::AlreadyHappened, Duration::Exact(_)) => Instant::AlreadyHappened,
            (Instant::AlreadyHappened, Duration::NotHappening) => Instant::AlreadyHappened,
            (Instant::Exact(v1), Duration::Exact(v2)) => Instant::Exact(v1.add(v2)),
            (Instant::Exact(_), Duration::NotHappening) => Instant::NotHappening,
            (Instant::NotHappening, Duration::Exact(_)) => Instant::NotHappening,
            (Instant::NotHappening, Duration::NotHappening) => Instant::NotHappening,
        }
    }
}

impl Div<u32> for Duration {
    type Output = Duration;

    fn div(self, rhs: u32) -> Self::Output {
        match self {
            Duration::Exact(d) => Duration::Exact(d / rhs),
            Duration::NotHappening => Duration::NotHappening,
        }
    }
}

impl PartialOrd for Instant {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Self::cmp(self, other))
    }
}

impl Ord for Instant {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Instant::AlreadyHappened, Instant::AlreadyHappened) => Ordering::Equal,
            (Instant::AlreadyHappened, Instant::Exact(_)) => Ordering::Less,
            (Instant::AlreadyHappened, Instant::NotHappening) => Ordering::Less,
            (Instant::Exact(_), Instant::AlreadyHappened) => Ordering::Greater,
            (Instant::Exact(v1), Instant::Exact(v2)) => v1.cmp(v2),
            (Instant::Exact(_), Instant::NotHappening) => Ordering::Less,
            (Instant::NotHappening, Instant::AlreadyHappened) => Ordering::Greater,
            (Instant::NotHappening, Instant::Exact(_)) => Ordering::Greater,
            (Instant::NotHappening, Instant::NotHappening) => Ordering::Equal,
        }
    }
}

impl PartialOrd for Duration {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Self::cmp(self, other))
    }
}

impl Ord for Duration {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Duration::Exact(v1), Duration::Exact(v2)) => v1.cmp(v2),
            (Duration::Exact(_), Duration::NotHappening) => Ordering::Less,
            (Duration::NotHappening, Duration::Exact(_)) => Ordering::Greater,
            (Duration::NotHappening, Duration::NotHappening) => Ordering::Equal,
        }
    }
}

impl From<std::time::Duration> for Duration {
    fn from(value: std::time::Duration) -> Self {
        Self::Exact(value)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn time_ord() {
        assert!(Instant::AlreadyHappened < Instant::now());
        assert!(Instant::now() < Instant::NotHappening);
        assert!(Instant::AlreadyHappened < Instant::NotHappening);
    }
}
