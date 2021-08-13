use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read};
use std::sync::Mutex;

use crate::stream::Stream;
use crate::unit::Unit;
use crate::Proxy;

use log::debug;
use url::Url;

/// Holder of recycled connections.
///
/// For each PoolKey (approximately hostname and port), there may be
/// multiple connections stored in the `recycle` map. If so, they are stored in
/// order from oldest at the front to freshest at the back.
///
/// The `lru` VecDeque is a companion struct to `recycle`, and is used to keep
/// track of which connections to expire if the pool is full on the next insert.
/// A given PoolKey can occur in lru multiple times. The first entry in lru for
/// a key K represents the first entry in `recycle[K]`. The second entry in lru
/// for `K` represents the second entry in `recycle[K]`, and so on. In other
/// words, `lru` is ordered the same way as the VecDeque entries in `recycle`:
/// oldest at the front, freshest at the back. This allows keeping track of which
/// host should have its connection dropped next.
///
/// These invariants hold at the start and end of each method:
///  - The length `lru` is equal to the sum of lengths of `recycle`'s VecDeques.
///  - Each PoolKey exists the same number of times in `lru` as it has entries in `recycle`.
///  - If there is an entry in `recycle`, it has at least one element.
///  - The length of `lru` is less than or equal to max_idle_connections.
///  - The length of recycle[K] is less than or equal to max_idle_connections_per_host.
///
/// *Internal API*
pub(crate) struct ConnectionPool {
    inner: Mutex<Inner>,
    max_idle_connections: usize,
    max_idle_connections_per_host: usize,
}

struct Inner {
    // the actual pooled connection. however only one per hostname:port.
    recycle: HashMap<PoolKey, VecDeque<Stream>>,
    // This is used to keep track of which streams to expire when the
    // pool reaches MAX_IDLE_CONNECTIONS. The corresponding PoolKeys for
    // recently used Streams are added to the back of the queue;
    // old streams are removed from the front.
    lru: VecDeque<PoolKey>,
}

impl fmt::Debug for ConnectionPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionPool")
            .field("max_idle", &self.max_idle_connections)
            .field("max_idle_per_host", &self.max_idle_connections_per_host)
            .field("connections", &self.inner.lock().unwrap().lru.len())
            .finish()
    }
}
fn remove_first_match(list: &mut VecDeque<PoolKey>, key: &PoolKey) -> Option<PoolKey> {
    match list.iter().position(|x| x == key) {
        Some(i) => list.remove(i),
        None => None,
    }
}

fn remove_last_match(list: &mut VecDeque<PoolKey>, key: &PoolKey) -> Option<PoolKey> {
    match list.iter().rposition(|x| x == key) {
        Some(i) => list.remove(i),
        None => None,
    }
}

impl ConnectionPool {
    pub(crate) fn new_with_limits(
        max_idle_connections: usize,
        max_idle_connections_per_host: usize,
    ) -> Self {
        ConnectionPool {
            inner: Mutex::new(Inner {
                recycle: HashMap::new(),
                lru: VecDeque::new(),
            }),
            max_idle_connections,
            max_idle_connections_per_host,
        }
    }

    /// Return true if either of the max_* settings is 0, meaning we should do no work.
    fn noop(&self) -> bool {
        self.max_idle_connections == 0 || self.max_idle_connections_per_host == 0
    }

    /// How the unit::connect tries to get a pooled connection.
    pub fn try_get_connection(&self, url: &Url, proxy: Option<Proxy>) -> Option<Stream> {
        let key = PoolKey::new(url, proxy);
        self.remove(&key)
    }

    fn remove(&self, key: &PoolKey) -> Option<Stream> {
        let mut inner = self.inner.lock().unwrap();
        match inner.recycle.entry(key.clone()) {
            Entry::Occupied(mut occupied_entry) => {
                let streams = occupied_entry.get_mut();
                // Take the newest stream.
                let stream = streams.pop_back();
                let stream = stream.expect("invariant failed: empty VecDeque in `recycle`");

                if streams.is_empty() {
                    occupied_entry.remove();
                }

                // Remove the newest matching PoolKey from self.lru. That
                // corresponds to the stream we just removed from `recycle`.
                remove_last_match(&mut inner.lru, key)
                    .expect("invariant failed: key in recycle but not in lru");

                debug!("pulling stream from pool: {:?} -> {:?}", key, stream);
                Some(stream)
            }
            Entry::Vacant(_) => None,
        }
    }

    fn add(&self, key: PoolKey, stream: Stream) {
        if self.noop() {
            return;
        }
        debug!("adding stream to pool: {:?} -> {:?}", key, stream);

        let mut inner = self.inner.lock().unwrap();
        match inner.recycle.entry(key.clone()) {
            Entry::Occupied(mut occupied_entry) => {
                let streams = occupied_entry.get_mut();
                streams.push_back(stream);
                if streams.len() > self.max_idle_connections_per_host {
                    // Remove the oldest entry
                    let stream = streams.pop_front().expect("empty streams list");
                    debug!(
                        "host {:?} has {} conns, dropping oldest: {:?}",
                        key,
                        streams.len(),
                        stream
                    );
                    remove_first_match(&mut inner.lru, &key)
                        .expect("invariant failed: key in recycle but not in lru");
                }
            }
            Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(vec![stream].into());
            }
        }
        inner.lru.push_back(key);
        if inner.lru.len() > self.max_idle_connections {
            drop(inner);
            self.remove_oldest()
        }
    }

    /// Find the oldest stream in the pool. Remove its representation from lru,
    /// and the stream itself from `recycle`. Drops the stream, which closes it.
    fn remove_oldest(&self) {
        assert!(!self.noop(), "remove_oldest called on Pool with max of 0");
        let mut inner = self.inner.lock().unwrap();
        let key = inner.lru.pop_front();
        let key = key.expect("tried to remove oldest but no entries found!");
        match inner.recycle.entry(key) {
            Entry::Occupied(mut occupied_entry) => {
                let streams = occupied_entry.get_mut();
                let stream = streams
                    .pop_front()
                    .expect("invariant failed: key existed in recycle but no streams available");
                debug!("dropping oldest stream in pool: {:?}", stream);
                if streams.is_empty() {
                    occupied_entry.remove();
                }
            }
            Entry::Vacant(_) => panic!("invariant failed: key existed in lru but not in recycle"),
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().lru.len()
    }
}

#[derive(PartialEq, Clone, Eq, Hash)]
struct PoolKey {
    scheme: String,
    hostname: String,
    port: Option<u16>,
    proxy: Option<Proxy>,
}

use std::fmt;

impl fmt::Debug for PoolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "{}|{}|{}",
            self.scheme,
            self.hostname,
            self.port.unwrap_or(0)
        ))
    }
}

impl PoolKey {
    fn new(url: &Url, proxy: Option<Proxy>) -> Self {
        let port = url.port_or_known_default();
        PoolKey {
            scheme: url.scheme().to_string(),
            hostname: url.host_str().unwrap_or("").to_string(),
            port,
            proxy,
        }
    }
}

/// Read wrapper that returns the stream to the pool once the
/// read is exhausted (reached a 0).
///
/// *Internal API*
pub(crate) struct PoolReturnRead<R: Read + Sized + Into<Stream>> {
    // unit that contains the agent where we want to return the reader.
    unit: Option<Box<Unit>>,
    // wrapped reader around the same stream
    reader: Option<R>,
}

impl<R: Read + Sized + Into<Stream>> PoolReturnRead<R> {
    pub fn new(unit: Option<Box<Unit>>, reader: R) -> Self {
        PoolReturnRead {
            unit,
            reader: Some(reader),
        }
    }

    fn return_connection(&mut self) -> io::Result<()> {
        // guard we only do this once.
        if let (Some(unit), Some(reader)) = (self.unit.take(), self.reader.take()) {
            // bring back stream here to either go into pool or dealloc
            let mut stream = reader.into();
            if !stream.is_poolable() {
                // just let it deallocate
                return Ok(());
            }

            // ensure stream can be reused
            stream.reset()?;

            // insert back into pool
            let key = PoolKey::new(&unit.url, unit.agent.config.proxy.clone());
            unit.agent.state.pool.add(key, stream);
        }

        Ok(())
    }

    fn do_read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.reader.as_mut() {
            None => Ok(0),
            Some(reader) => reader.read(buf),
        }
    }
}

impl<R: Read + Sized + Into<Stream>> Read for PoolReturnRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let amount = self.do_read(buf)?;
        // only if the underlying reader is exhausted can we send a new
        // request to the same socket. hence, we only return it now.
        if amount == 0 {
            self.return_connection()?;
        }
        Ok(amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poolkey_new() {
        // Test that PoolKey::new() does not panic on unrecognized schemes.
        PoolKey::new(&Url::parse("zzz:///example.com").unwrap(), None);
    }

    #[test]
    fn pool_connections_limit() {
        // Test inserting connections with different keys into the pool,
        // filling and draining it. The pool should evict earlier connections
        // when the connection limit is reached.
        let pool = ConnectionPool::new_with_limits(10, 1);
        let hostnames = (0..pool.max_idle_connections * 2).map(|i| format!("{}.example", i));
        let poolkeys = hostnames.map(|hostname| PoolKey {
            scheme: "https".to_string(),
            hostname,
            port: Some(999),
            proxy: None,
        });
        for key in poolkeys.clone() {
            pool.add(key, Stream::from_vec(vec![]))
        }
        assert_eq!(pool.len(), pool.max_idle_connections);

        for key in poolkeys.skip(pool.max_idle_connections) {
            let result = pool.remove(&key);
            assert!(result.is_some(), "expected key was not in pool");
        }
        assert_eq!(pool.len(), 0)
    }

    #[test]
    fn pool_per_host_connections_limit() {
        // Test inserting connections with the same key into the pool,
        // filling and draining it. The pool should evict earlier connections
        // when the per-host connection limit is reached.
        let pool = ConnectionPool::new_with_limits(10, 2);
        let poolkey = PoolKey {
            scheme: "https".to_string(),
            hostname: "example.com".to_string(),
            port: Some(999),
            proxy: None,
        };

        for _ in 0..pool.max_idle_connections_per_host * 2 {
            pool.add(poolkey.clone(), Stream::from_vec(vec![]))
        }
        assert_eq!(pool.len(), pool.max_idle_connections_per_host);

        for _ in 0..pool.max_idle_connections_per_host {
            let result = pool.remove(&poolkey);
            assert!(result.is_some(), "expected key was not in pool");
        }
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn pool_checks_proxy() {
        // Test inserting different poolkeys with same address but different proxies.
        // Each insertion should result in an additional entry in the pool.
        let pool = ConnectionPool::new_with_limits(10, 1);
        let url = Url::parse("zzz:///example.com").unwrap();

        pool.add(PoolKey::new(&url, None), Stream::from_vec(vec![]));
        assert_eq!(pool.len(), 1);

        pool.add(
            PoolKey::new(&url, Some(Proxy::new("localhost:9999").unwrap())),
            Stream::from_vec(vec![]),
        );
        assert_eq!(pool.len(), 2);

        pool.add(
            PoolKey::new(
                &url,
                Some(Proxy::new("user:password@localhost:9999").unwrap()),
            ),
            Stream::from_vec(vec![]),
        );
        assert_eq!(pool.len(), 3);
    }
}
