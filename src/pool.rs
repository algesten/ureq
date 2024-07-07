use std::net::SocketAddr;
use std::time::Duration;

use http::Uri;

use crate::transport::{Buffers, Socket, Transport};
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
    pub fn borrow_buffers(&mut self) -> Buffers {
        self.conn.borrow_buffers()
    }

    pub fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error> {
        self.conn.transmit_output(amount, timeout)
    }

    pub fn await_input(&mut self, timeout: Duration, is_body: bool) -> Result<Buffers, Error> {
        self.conn.await_input(timeout, is_body)
    }

    pub fn consume_input(&mut self, amount: usize) {
        self.conn.consume_input(amount)
    }
}
