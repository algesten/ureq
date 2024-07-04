use hoot::client::flow::state::{Prepare, SendRequest};
use hoot::client::flow::{Flow, SendRequestResult};
use http::{Request, Response};

use crate::body::RecvBody;
use crate::pool::{Connection, ConnectionPool};
use crate::{Body, Error};

pub(crate) struct Unit;

impl Unit {
    pub(crate) fn run(
        &mut self,
        pool: &mut ConnectionPool,
        request: &Request<impl Body>,
    ) -> Result<Response<RecvBody>, Error> {
        let flow = Flow::new(request)?;

        self.prepare(pool, flow)?;

        todo!()
    }

    fn prepare<B>(
        &mut self,
        pool: &mut ConnectionPool,
        flow: Flow<B, Prepare>,
    ) -> Result<(), Error> {
        let connection = pool.connect(flow.uri())?;

        self.send_request(flow.proceed(), connection)
    }

    fn send_request<B>(
        &self,
        mut flow: Flow<B, SendRequest>,
        mut connection: Connection,
    ) -> Result<(), Error> {
        loop {
            if flow.can_proceed() {
                break;
            }

            let mut buffer = connection.output_buffer();
            let n = flow.write(buffer.as_mut())?;
            buffer.flush(n)?;
        }

        match flow.proceed().unwrap() {
            SendRequestResult::Await100(_) => todo!(),
            SendRequestResult::SendBody(_) => todo!(),
            SendRequestResult::RecvResponse(_) => todo!(),
        }
    }
}
