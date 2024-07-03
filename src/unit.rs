use hoot::client::flow::state::{Prepare, SendRequest};
use hoot::client::flow::{Flow, SendRequestResult};
use http::{Request, Response};

use crate::agent::{ConnectionPool, Transport};
use crate::body::RecvBody;
use crate::{Body, Error};

pub(crate) struct Unit;

impl Unit {
    pub(crate) fn handle(
        &mut self,
        pool: &mut dyn ConnectionPool,
        request: &Request<impl Body>,
    ) -> Result<Response<RecvBody>, Error> {
        let flow = Flow::new(request)?;

        self.prepare(pool, flow)?;

        todo!()
    }

    fn prepare<'a, B>(
        &mut self,
        pool: &mut dyn ConnectionPool,
        flow: Flow<'a, B, Prepare>,
    ) -> Result<(), Error> {
        let transport = pool.acquire(flow.uri())?;

        self.send_request(flow.proceed(), transport)
    }

    fn send_request<B>(
        &self,
        mut flow: Flow<B, SendRequest>,
        transport: &mut dyn Transport,
    ) -> Result<(), Error> {
        loop {
            if flow.can_proceed() {
                break;
            }

            let buffer = transport.output_buffer();
            let n = flow.write(buffer.as_mut())?;
            buffer.push_output(n)?;
        }

        match flow.proceed().unwrap() {
            SendRequestResult::Await100(_) => todo!(),
            SendRequestResult::SendBody(_) => todo!(),
            SendRequestResult::RecvResponse(_) => todo!(),
        }
    }
}
