use http::Uri;

use crate::transport::{Conn, Transport};
use crate::Error;

#[derive(Debug)]
pub(crate) struct ConnectionPool {
    connector: Box<dyn Transport>,
}

impl ConnectionPool {
    pub fn new(connector: impl Transport) -> Self {
        ConnectionPool {
            connector: Box::new(connector),
        }
    }

    pub fn connect(&mut self, uri: &Uri) -> Result<Connection, Error> {
        Ok(Connection {
            conn: self.connector.connect(uri)?,
        })
    }
}

pub(crate) struct Connection {
    conn: Box<dyn Conn>,
}

impl Connection {
    pub fn output_buffer(&mut self) -> OutputBuffer {
        todo!()
    }
}

pub(crate) struct OutputBuffer<'a> {
    transport: &'a mut dyn Conn,
}

impl<'a> OutputBuffer<'a> {
    pub fn flush(self, amount: usize) -> Result<(), Error> {
        self.transport.output_buffer_flush(amount)
    }
}

impl<'a> AsMut<[u8]> for OutputBuffer<'a> {
    fn as_mut(&mut self) -> &mut [u8] {
        self.transport.output_buffer()
    }
}
