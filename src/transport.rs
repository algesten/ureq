use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::Duration;

use http::Uri;

use crate::Error;

pub trait Transport: Debug + 'static {
    fn connect(
        &mut self,
        uri: &Uri,
        addr: SocketAddr,
        timeout: Duration,
    ) -> Result<Box<dyn Socket>, Error>;
}

pub trait Socket: Debug {
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
