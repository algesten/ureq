use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::Duration;

use http::Uri;

use crate::proxy::Proxy;
use crate::resolver::Resolver;
use crate::Error;

pub trait Connector: Debug + 'static {
    fn connect(&self, details: &ConnectionDetails) -> Result<Box<dyn Transport>, Error>;
}

pub struct ConnectionDetails<'a> {
    pub uri: &'a Uri,
    pub addr: SocketAddr,
    pub proxy: &'a Option<Proxy>,
    pub resolver: &'a dyn Resolver,
    pub timeout: Duration,
}

pub trait Transport: Debug {
    fn borrow_buffers(&mut self) -> Buffers;
    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error>;
    fn await_input(&mut self, timeout: Duration, is_body: bool) -> Result<Buffers, Error>;
    fn consume_input(&mut self, amount: usize);
}

pub struct Buffers<'a> {
    pub input: &'a mut [u8],
    pub output: &'a mut [u8],
}

impl Buffers<'_> {
    pub(crate) fn empty() -> Buffers<'static> {
        Buffers {
            input: &mut [],
            output: &mut [],
        }
    }
}

#[derive(Debug)]
pub struct DefaultConnector;

impl Connector for DefaultConnector {
    fn connect(&self, _details: &ConnectionDetails) -> Result<Box<dyn Transport>, Error> {
        todo!()
    }
}
