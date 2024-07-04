use std::net::SocketAddr;
use std::time::Instant;

use hoot::client::flow::Flow;
use http::{Request, Uri};

use crate::flow::FlowHolder;
use crate::{Body, Error};

pub(crate) struct Unit<'a, B> {
    start_time: Instant,
    flow: FlowHolder<'a, B>,
}

pub enum Output<'a> {
    Reset,
    DnsLookup {
        uri: &'a Uri,
        timeout: Instant,
    },
    OpenConnection {
        uri: &'a Uri,
        addr: SocketAddr,
        timeout: Instant,
    },
    Transmit {
        amount: usize,
        timeout: Instant,
    },
    AwaitInput {
        timeout: Instant,
    },
}

pub enum Input<'a> {
    SocketAddr(SocketAddr),
    ConnectionOpen,
    Input { data: &'a [u8] },
}

impl<'a, B: Body> Unit<'a, B> {
    pub fn new(start_time: Instant, request: &'a Request<B>) -> Result<Self, Error> {
        let flow = Flow::new(request)?;

        let flow = FlowHolder::Prepare(flow);

        Ok(Self { start_time, flow })
    }

    pub fn poll_output(&'a mut self, transmit_buffer: &mut [u8]) -> Output {
        todo!()
    }

    pub fn handle_input(&mut self, now: Instant, input: Input) {}
}
