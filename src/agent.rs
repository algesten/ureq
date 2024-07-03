use std::fmt::Debug;

use http::{Request, Response, Uri};

use crate::body::RecvBody;
use crate::unit::Unit;
use crate::{Body, Error};

#[derive(Debug)]
pub struct Agent {
    pool: Box<dyn ConnectionPool>,
}

impl Agent {
    pub fn new(pool: impl ConnectionPool) -> Self {
        Agent {
            pool: Box::new(pool),
        }
    }

    pub(crate) fn new_default() -> Self {
        Agent::new(RustlConnectionPool)
    }

    // TODO(martin): Can we improve this signature? The ideal would be:
    // fn run(&self, request: &Request<impl Body>) -> Result<Response<impl Body>, Error>

    // TODO(martin): One design idea is to be able to create requests in one thread, then
    // actually run them to completion in another. &mut self here makes it impossible to use
    // Agent in such a design. Is that a concern?
    pub(crate) fn run(
        &mut self,
        request: &Request<impl Body>,
    ) -> Result<Response<RecvBody>, Error> {
        let response = Unit.handle(&mut *self.pool, request)?;
        Ok(response)
    }
}

pub trait ConnectionPool: Debug + 'static {
    fn acquire(&mut self, uri: &Uri) -> Result<&mut dyn Transport, Error>;
}

pub trait Transport: Debug {
    fn output_buffer(&mut self) -> &mut dyn OutputBuffer;
}

pub trait OutputBuffer: AsMut<[u8]> {
    fn push_output(&mut self, amount: usize) -> Result<(), Error>;
}

#[derive(Debug)]
pub struct RustlConnectionPool;

impl ConnectionPool for RustlConnectionPool {
    fn acquire(&mut self, _uri: &Uri) -> Result<&mut dyn Transport, Error> {
        todo!()
    }
}
