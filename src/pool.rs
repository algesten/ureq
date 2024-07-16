use core::fmt;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};

use http::uri::{Authority, Scheme};
use http::Uri;

use crate::proxy::Proxy;
use crate::time::{NextTimeout, Duration, Instant};
use crate::transport::{Buffers, ConnectionDetails, Connector, Transport};
use crate::util::DebugAuthority;
use crate::{AgentConfig, Error};

pub(crate) struct ConnectionPool {
    connector: Box<dyn Connector>,
    pool: Arc<Mutex<Pool>>,
}

impl ConnectionPool {
    pub fn new(connector: impl Connector, config: &AgentConfig) -> Self {
        ConnectionPool {
            connector: Box::new(connector),
            pool: Arc::new(Mutex::new(Pool::new(config))),
        }
    }

    pub fn connect(&self, details: &ConnectionDetails) -> Result<Connection, Error> {
        let key = PoolKey::new(details.uri, details.proxy);

        {
            let mut pool = self.pool.lock().unwrap();
            pool.purge(details.now);

            if let Some(conn) = pool.get(&key) {
                debug!("Use pooled: {:?}", key);
                return Ok(conn);
            }
        }

        let transport = self
            .connector
            .connect(details, None)?
            .ok_or(Error::ConnectionFailed)?;

        let conn = Connection {
            transport,
            key,
            last_use: details.now,
            pool: Arc::downgrade(&self.pool),
            position_per_host: None,
        };

        Ok(conn)
    }
}

pub(crate) struct Connection {
    transport: Box<dyn Transport>,
    key: PoolKey,
    last_use: Instant,
    pool: Weak<Mutex<Pool>>,

    /// Used to prune max_idle_connections_by_host.
    ///
    /// # Example
    ///
    /// If we have a max idle per hosts set to 3, and we have the following LRU:
    ///
    /// ```text
    /// [B, A, A, B, A, B, A]
    /// ```
    ///
    /// This field is used to enumerate the elements per host reverse:
    ///
    /// ```text
    /// [B2, A3, A2, B1, A1, B0, A0]
    /// ```
    ///
    /// Once we have that enumeration, we can drop elements from the front where there
    /// position_per_host >= idle_per_host.
    position_per_host: Option<usize>,
}

impl Connection {
    pub fn buffers(&mut self) -> &mut dyn Buffers {
        self.transport.buffers()
    }

    pub fn transmit_output(
        &mut self,
        amount: usize,
        timeout: NextTimeout,
    ) -> Result<(), Error> {
        self.transport.transmit_output(amount, timeout)
    }

    pub fn await_input(&mut self, timeout: NextTimeout) -> Result<(), Error> {
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
        self.last_use = now;

        let Some(arc) = self.pool.upgrade() else {
            debug!("Pool gone: {:?}", self.key);
            return;
        };

        debug!("Return to pool: {:?}", self.key);

        let mut pool = arc.lock().unwrap();

        pool.add(self);
        pool.purge(now);
    }

    fn age(&self, now: Instant) -> Duration {
        now.duration_since(now)
    }

    fn is_open(&mut self) -> bool {
        self.transport.is_open()
    }
}

/// The pool key is the Scheme, Authority from the uri and the Proxy setting
///
///
/// ```notrust
/// abc://username:password@example.com:123/path/data?key=value&key2=value2#fragid1
/// |-|   |-------------------------------||--------| |-------------------| |-----|
///  |                  |                       |               |              |
/// scheme          authority                 path            query         fragment
/// ```
///
/// It's correct to include username/password since connections with differing such and
/// the same host/port must not be mixed up.
///
#[derive(Clone, PartialEq, Eq)]
struct PoolKey(Arc<PoolKeyInner>);

impl PoolKey {
    fn new(uri: &Uri, proxy: &Option<Proxy>) -> Self {
        let inner = PoolKeyInner(
            uri.scheme().expect("uri with scheme").clone(),
            uri.authority().expect("uri with authority").clone(),
            proxy.clone(),
        );

        PoolKey(Arc::new(inner))
    }
}

#[derive(PartialEq, Eq)]
struct PoolKeyInner(Scheme, Authority, Option<Proxy>);

#[derive(Debug)]
struct Pool {
    lru: VecDeque<Connection>,
    max_idle_connections: usize,
    max_idle_connections_per_host: usize,
    max_idle_age: Duration,
}

impl Pool {
    fn new(config: &AgentConfig) -> Self {
        Pool {
            lru: VecDeque::new(),
            max_idle_connections: config.max_idle_connections,
            max_idle_connections_per_host: config.max_idle_connections_per_host,
            max_idle_age: config.max_idle_age,
        }
    }

    fn purge(&mut self, now: Instant) {
        while self.lru.len() > self.max_idle_connections || self.front_is_too_old(now) {
            self.lru.pop_front();
        }

        self.update_position_per_host();

        let max = self.max_idle_connections_per_host;

        // unwrap is ok because update_position_per_host() should have set all
        self.lru.retain(|c| c.position_per_host.unwrap() < max);
    }

    fn front_is_too_old(&self, now: Instant) -> bool {
        self.lru.front().map(|c| c.age(now)) > Some(self.max_idle_age)
    }

    fn update_position_per_host(&mut self) {
        // Reset position counters
        for c in &mut self.lru {
            c.position_per_host = None;
        }

        loop {
            let maybe_uncounted = self
                .lru
                .iter()
                .rev()
                .find(|c| c.position_per_host.is_none());

            let Some(uncounted) = maybe_uncounted else {
                break; // nothing more to count.
            };

            let key_to_count = uncounted.key.clone();

            for (position, c) in self
                .lru
                .iter_mut()
                .rev()
                .filter(|c| c.key == key_to_count)
                .enumerate()
            {
                c.position_per_host = Some(position);
            }
        }
    }

    fn add(&mut self, conn: Connection) {
        self.lru.push_back(conn)
    }

    fn get(&mut self, key: &PoolKey) -> Option<Connection> {
        while let Some(i) = self.lru.iter().position(|c| c.key == *key) {
            let mut conn = self.lru.remove(i).unwrap(); // unwrap ok since we just got the position

            // Before we release the connection, we probe that it appears to still work.
            if !conn.is_open() {
                // This connection is broken. Try find another one.
                continue;
            }

            return Some(conn);
        }
        None
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

impl fmt::Debug for PoolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolKey")
            .field("scheme", &self.0 .0)
            .field("authority", &DebugAuthority(&self.0 .1))
            .field("proxy", &self.0 .2)
            .finish()
    }
}
