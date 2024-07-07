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
    fn buffer_borrow(&mut self) -> &mut [u8];
    fn buffer_transmit(&mut self, amount: usize, timeout: Duration) -> Result<(), Error>;
    fn input_await(&mut self, timeout: Duration) -> Result<&[u8], Error>;
    fn input_consume(&mut self, amount: usize);
}
