use std::net::SocketAddr;
use std::time::Duration;

use http::Uri;

use crate::transport::{Socket, Transport};
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

    pub fn connect(
        &mut self,
        uri: &Uri,
        addr: SocketAddr,
        timeout: Duration,
    ) -> Result<Connection, Error> {
        Ok(Connection {
            conn: self.connector.connect(uri, addr, timeout)?,
        })
    }
}

pub(crate) struct Connection {
    conn: Box<dyn Socket>,
}

impl Connection {
    pub fn buffer_borrow(&mut self) -> &mut [u8] {
        self.conn.buffer_borrow()
    }

    pub fn buffer_transmit(&mut self, amount: usize, timeout: Duration) -> Result<(), Error> {
        self.conn.buffer_transmit(amount, timeout)
    }

    pub fn input_await(&mut self, timeout: Duration) -> Result<&[u8], Error> {
        self.conn.input_await(timeout)
    }

    pub fn input_consume(&mut self, amount: usize) {
        self.conn.input_consume(amount)
    }
}
