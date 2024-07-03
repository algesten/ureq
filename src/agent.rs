use http::{Request, Response};

use crate::body::RecvBody;
use crate::{Body, Error};

#[derive(Debug)]
pub struct Agent {}
impl Agent {
    pub(crate) fn new() -> Self {
        Self {}
    }

    // TODO(martin): Can we improve this signature? The ideal would be:
    // fn run(&self, request: &Request<impl Body>) -> Result<Response<impl Body>, Error>
    pub(crate) fn run(&self, _request: &Request<impl Body>) -> Result<Response<RecvBody>, Error> {
        Ok(Response::new(RecvBody))
    }
}
