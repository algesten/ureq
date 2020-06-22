use std::collections::{HashMap, VecDeque};
use std::io::{Read, Result as IoResult};

use crate::stream::Stream;
use crate::unit::Unit;

use url::Url;

pub const DEFAULT_HOST: &str = "localhost";
const MAX_IDLE_CONNECTIONS: usize = 100;

/// Holder of recycled connections.
///
/// *Internal API*
#[derive(Default, Debug)]
pub(crate) struct ConnectionPool {
    // the actual pooled connection. however only one per hostname:port.
    recycle: HashMap<PoolKey, Stream>,
    // This is used to keep track of which streams to expire when the
    // pool reaches MAX_IDLE_CONNECTIONS. The corresponding PoolKey for
    // recently used Streams are added to the back of the queue;
    // old streams are removed from the front.
    // Invariant: The length of recycle and lru are the same.
    // Invariant: Every PoolKey exists as a key in recycle, and vice versa.
    lru: VecDeque<PoolKey>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        ConnectionPool {
            ..Default::default()
        }
    }

    /// How the unit::connect tries to get a pooled connection.
    pub fn try_get_connection(&mut self, url: &Url) -> Option<Stream> {
        let key = PoolKey::new(url);
        if !self.recycle.contains_key(&key) {
            return None;
        }
        let index = self.lru.iter().position(|k| k == &key);
        assert!(
            index.is_some(),
            "invariant failed: key existed in recycle but not lru"
        );
        self.lru.remove(index.unwrap());
        self.recycle.remove(&key)
    }

    fn add(&mut self, key: PoolKey, stream: Stream) {
        if self.recycle.len() + 1 > MAX_IDLE_CONNECTIONS {
            self.remove_oldest();
        }
        self.lru.push_back(key.clone());
        self.recycle.insert(key, stream);
    }

    fn remove_oldest(&mut self) {
        if let Some(key) = self.lru.pop_front() {
            assert!(
                self.recycle.contains_key(&key),
                "invariant failed: key existed in lru but not in recycle"
            );
            self.recycle.remove(&key);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.recycle.len()
    }

    #[cfg(all(test, any(feature = "tls", feature = "native-tls")))]
    pub fn get(&self, hostname: &str, port: u16) -> Option<&Stream> {
        let key = PoolKey {
            hostname: hostname.into(),
            port,
        };
        self.recycle.get(&key)
    }
}

#[derive(Debug, PartialEq, Clone, Eq, Hash)]
struct PoolKey {
    hostname: String,
    port: u16,
}

impl PoolKey {
    fn new(url: &Url) -> Self {
        let port = if cfg!(test) {
            if let Some(p) = url.port_or_known_default() {
                Some(p)
            } else if url.scheme() == "test" {
                Some(42)
            } else {
                None
            }
        } else {
            url.port_or_known_default()
        };
        PoolKey {
            hostname: url.host_str().unwrap_or(DEFAULT_HOST).into(),
            port: port.expect("Failed to get port for pool key"),
        }
    }
}

/// Read wrapper that returns the stream to the pool once the
/// read is exhausted (reached a 0).
///
/// *Internal API*
pub(crate) struct PoolReturnRead<R: Read + Sized + Into<Stream>> {
    // unit that contains the agent where we want to return the reader.
    unit: Option<Unit>,
    // wrapped reader around the same stream
    reader: Option<R>,
}

impl<R: Read + Sized + Into<Stream>> PoolReturnRead<R> {
    pub fn new(unit: Option<Unit>, reader: R) -> Self {
        PoolReturnRead {
            unit,
            reader: Some(reader),
        }
    }

    fn return_connection(&mut self) {
        // guard we only do this once.
        if let (Some(unit), Some(reader)) = (self.unit.take(), self.reader.take()) {
            let state = &mut unit.agent.lock().unwrap();
            // bring back stream here to either go into pool or dealloc
            let stream = reader.into();
            if let Some(agent) = state.as_mut() {
                if !stream.is_poolable() {
                    // just let it deallocate
                    return;
                }
                // insert back into pool
                let key = PoolKey::new(&unit.url);
                agent.pool().add(key, stream);
            }
        }
    }

    fn do_read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match self.reader.as_mut() {
            None => Ok(0),
            Some(reader) => reader.read(buf),
        }
    }
}

impl<R: Read + Sized + Into<Stream>> Read for PoolReturnRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let amount = self.do_read(buf)?;
        // only if the underlying reader is exhausted can we send a new
        // request to the same socket. hence, we only return it now.
        if amount == 0 {
            self.return_connection();
        }
        Ok(amount)
    }
}

impl<R: Read + Sized + Into<Stream>> Drop for PoolReturnRead<R> {
    fn drop(&mut self) {
        self.return_connection();
    }
}
