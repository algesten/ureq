use std::fmt::Debug;

use http::{Request, Response, Uri};

use crate::body::RecvBody;
use crate::pool::ConnectionPool;
use crate::transport::{Conn, Transport};
use crate::unit::Unit;
use crate::{Body, Error};

#[derive(Debug)]
pub struct Agent {
    pool: ConnectionPool,
}

impl Agent {
    pub fn new(pool: impl Transport) -> Self {
        Agent {
            pool: ConnectionPool::new(pool),
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
        let response = Unit.run(&mut self.pool, request)?;
        Ok(response)
    }
}

#[derive(Debug)]
pub struct RustlConnectionPool;

impl Transport for RustlConnectionPool {
    fn connect(&mut self, _uri: &Uri) -> Result<&mut dyn Conn, Error> {
        todo!()
    }
}
