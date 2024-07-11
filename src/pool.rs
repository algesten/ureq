use core::fmt;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use http::uri::Authority;
use http::Uri;

use crate::time::{Duration, Instant};
use crate::transport::{Buffers, ConnectionDetails, Connector, Transport};
use crate::Error;

pub(crate) struct ConnectionPool {
    connector: Box<dyn Connector>,
    pool: Pool,
}

impl ConnectionPool {
    pub fn new(connector: impl Connector) -> Self {
        ConnectionPool {
            connector: Box::new(connector),
            pool: Pool::default(),
        }
    }

    pub fn connect(&self, details: &ConnectionDetails) -> Result<Connection, Error> {
        let key = PoolKey::from(details.uri);

        {
            let mut pool = self.pool.lock();
            pool.purge(
                details.now,
                details.config.max_idle_connections,
                details.config.max_idle_age,
            );

            if let Some(conn) = pool.get(&key) {
                debug!("Use pooled: {:?}", key);
                return Ok(conn);
            }
        }

        let transport = self
            .connector
            .connect(&details, None)?
            .ok_or(Error::ConnectionFailed)?;

        let conn = Connection {
            transport,
            key,
            last_use: details.now,
            pool: self.pool.clone(), // Cheap Arc clone
        };

        Ok(conn)
    }
}

pub(crate) struct Connection {
    transport: Box<dyn Transport>,
    key: PoolKey,
    last_use: Instant,
    pool: Pool,
}

impl Connection {
    pub fn buffers(&mut self) -> &mut dyn Buffers {
        self.transport.buffers()
    }

    pub fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error> {
        self.transport.transmit_output(amount, timeout)
    }

    pub fn await_input(&mut self, timeout: Duration) -> Result<(), Error> {
        self.transport.await_input(timeout)
    }

    pub fn consume_input(&mut self, amount: usize) {
        self.transport.consume_input(amount)
    }

    pub fn close(self) {
        debug!("Close: {:?}", self.key);
        // Just consume self.
    }

    pub fn reuse(mut self, now: Instant) {
        debug!("Return to pool: {:?}", self.key);
        let copy = self.pool.clone();
        let mut pool = copy.lock();
        self.last_use = now;
        pool.0.push_back(self);
    }

    fn age(&self, now: Instant) -> Duration {
        now.duration_since(now)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PoolKey(Authority);

#[derive(Debug, Default, Clone)]
struct Pool(Arc<Mutex<VecDeque<Connection>>>);

impl Pool {
    fn lock(&self) -> PoolLock {
        let lock = self.0.lock().unwrap();
        PoolLock(lock)
    }
}

struct PoolLock<'a>(MutexGuard<'a, VecDeque<Connection>>);

impl<'a> PoolLock<'a> {
    fn purge(&mut self, now: Instant, max_entries: usize, max_age: Duration) {
        while self.0.len() > max_entries || self.0.front().map(|c| c.age(now)) > Some(max_age) {
            self.0.pop_front();
        }
    }

    fn get(&mut self, key: &PoolKey) -> Option<Connection> {
        if let Some(i) = self.0.iter().position(|c| c.key == *key) {
            let conn = self.0.remove(i).unwrap(); // unwrap ok since we just got the position
            Some(conn)
        } else {
            None
        }
    }
}

impl From<&Uri> for PoolKey {
    fn from(uri: &Uri) -> Self {
        PoolKey(uri.authority().expect("uri with authority").clone())
    }
}

impl fmt::Debug for ConnectionPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionPool")
            .field("connector", &self.connector)
            .finish()
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("key", &self.key)
            .field("conn", &self.transport)
            .finish()
    }
}
