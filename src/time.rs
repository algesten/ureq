use std::cmp::Ordering;
use std::ops::{Add, AddAssign, Deref, Sub, SubAssign};
use std::time;

/// Wrapper for [`std::time::Instant`] that provides additional time points in the past or future
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instant {
    #[allow(dead_code)]
    AlreadyHappened,
    Exact(time::Instant),
    NotHappening,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duration {
    Exact(time::Duration),
    NotHappening,
}

impl Duration {
    const ZERO: Duration = Duration::Exact(time::Duration::ZERO);

    pub fn from_secs(secs: u64) -> Duration {
        Duration::Exact(time::Duration::from_secs(secs))
    }

    pub fn is_not_happening(&self) -> bool {
        *self == Duration::NotHappening
    }
}

impl Deref for Duration {
    type Target = time::Duration;

    fn deref(&self) -> &Self::Target {
        match self {
            Duration::Exact(v) => v,
            Duration::NotHappening => {
                const NOT_HAPPENING: time::Duration = time::Duration::from_secs(u64::MAX);
                &NOT_HAPPENING
            }
        }
    }
}

impl Instant {
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
        match self {
            Instant::Exact(v) => Instant::Exact(v.add(*rhs)),
            x => x,
        }
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, rhs: Duration) {
        if let Instant::Exact(v) = self {
            v.add_assign(*rhs)
        }
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;

    fn sub(self, rhs: Duration) -> Self::Output {
        match self {
            Instant::Exact(v) => Instant::Exact(v.sub(*rhs)),
            x => x,
        }
    }
}

impl SubAssign<Duration> for Instant {
    fn sub_assign(&mut self, rhs: Duration) {
        if let Instant::Exact(v) = self {
            v.sub_assign(*rhs)
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
