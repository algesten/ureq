//! Timeout utilities, mostly used during connecting.

use std::io;
use std::time::{Duration, Instant};

/// If the deadline is in the future, return the remaining time until
/// then. Otherwise return a TimedOut error.
pub fn time_until_deadline<S: Into<String>>(deadline: Instant, error: S) -> io::Result<Duration> {
    let now = Instant::now();
    match deadline.checked_duration_since(now) {
        None => Err(io_err_timeout(error.into())),
        Some(duration) => Ok(duration),
    }
}

pub fn io_err_timeout(error: String) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, error)
}
