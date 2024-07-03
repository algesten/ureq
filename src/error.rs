use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("this is bad")]
    Bad,
}
