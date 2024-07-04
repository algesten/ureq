use std::fmt::Debug;

use http::Uri;

use crate::Error;

pub trait Transport: Debug + 'static {
    fn connect(&mut self, uri: &Uri) -> Result<Box<dyn Conn>, Error>;
}

pub trait Conn: Debug {
    fn output_buffer(&mut self) -> &mut [u8];
    fn output_buffer_flush(&mut self, amount: usize) -> Result<(), Error>;
}
