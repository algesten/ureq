use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("protocol: {0}")]
    Protocol(#[from] hoot::Error),

    #[error("io: {0}")]
    Io(#[from] io::Error),
}
