use std::time::Duration;

use crate::transport::{Buffers, ConnectionDetails, Connector, Transport};
use crate::Error;

#[derive(Debug)]
pub(crate) struct ConnectionPool {
    connector: Box<dyn Connector>,
}

impl ConnectionPool {
    pub fn new(connector: impl Connector) -> Self {
        ConnectionPool {
            connector: Box::new(connector),
        }
    }

    pub fn connect(&self, details: &ConnectionDetails) -> Result<Connection, Error> {
        Ok(Connection {
            conn: self
                .connector
                .connect(&details, None)?
                .ok_or(Error::ConnectionFailed)?,
        })
    }
}

pub(crate) struct Connection {
    conn: Box<dyn Transport>,
}

impl Connection {
    pub fn buffers(&mut self) -> &mut dyn Buffers {
        self.conn.buffers()
    }

    pub fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error> {
        self.conn.transmit_output(amount, timeout)
    }

    pub fn await_input(&mut self, timeout: Duration) -> Result<(), Error> {
        self.conn.await_input(timeout)
    }

    pub fn consume_input(&mut self, amount: usize) {
        self.conn.consume_input(amount)
    }

    pub(crate) fn close(self) {
        //
    }

    pub(crate) fn reuse(self) {
        //
    }
}
