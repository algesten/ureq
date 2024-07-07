use std::{fmt, io};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Other(&'static str),

    #[error("protocol: {0}")]
    Protocol(#[from] hoot::Error),

    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("timeout: {0}")]
    Timeout(TimeoutReason),

    #[error("host not found")]
    HostNotFound,

    #[error("redirect failed")]
    RedirectFailed,
}

#[derive(Debug)]
pub enum TimeoutReason {
    Resolver,
    Global,
    Call,
}

impl fmt::Display for TimeoutReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeoutReason::Resolver => write!(f, "resolver"),
            TimeoutReason::Global => write!(f, "global"),
            TimeoutReason::Call => write!(f, "call"),
        }
    }
}
