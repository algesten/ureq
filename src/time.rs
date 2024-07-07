use std::cmp::Ordering;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::time::{self, Duration};

/// Wrapper for [`std::time::Instant`] that provides additional time points in the past or future
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Instant {
    AlreadyHappened,
    Exact(time::Instant),
    NotHappening,
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
            (Instant::Exact(v1), Instant::Exact(v2)) => v1.duration_since(v2),
            (Instant::Exact(_), Instant::AlreadyHappened) => Duration::from_secs(u64::MAX),
            (Instant::NotHappening, Instant::AlreadyHappened) => Duration::from_secs(u64::MAX),
            (Instant::NotHappening, Instant::Exact(_)) => Duration::from_secs(u64::MAX),
            (Instant::NotHappening, Instant::NotHappening) => Duration::from_secs(u64::MAX),
        }
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, rhs: Duration) -> Self::Output {
        match self {
            Instant::Exact(v) => Instant::Exact(v.add(rhs)),
            x @ _ => x,
        }
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, rhs: Duration) {
        match self {
            Instant::Exact(v) => v.add_assign(rhs),
            _ => {}
        }
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;

    fn sub(self, rhs: Duration) -> Self::Output {
        match self {
            Instant::Exact(v) => Instant::Exact(v.sub(rhs)),
            x @ _ => x,
        }
    }
}

impl SubAssign<Duration> for Instant {
    fn sub_assign(&mut self, rhs: Duration) {
        match self {
            Instant::Exact(v) => v.sub_assign(rhs),
            _ => {}
        }
    }
}

impl PartialOrd for Instant {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(match (self, other) {
            (Instant::AlreadyHappened, Instant::AlreadyHappened) => Ordering::Equal,
            (Instant::AlreadyHappened, Instant::Exact(_)) => Ordering::Less,
            (Instant::AlreadyHappened, Instant::NotHappening) => Ordering::Less,
            (Instant::Exact(_), Instant::AlreadyHappened) => Ordering::Greater,
            (Instant::Exact(v1), Instant::Exact(v2)) => v1.cmp(v2),
            (Instant::Exact(_), Instant::NotHappening) => Ordering::Less,
            (Instant::NotHappening, Instant::AlreadyHappened) => Ordering::Greater,
            (Instant::NotHappening, Instant::Exact(_)) => Ordering::Greater,
            (Instant::NotHappening, Instant::NotHappening) => Ordering::Equal,
        })
    }
}

impl Ord for Instant {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
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
