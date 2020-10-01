use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read};

use crate::stream::Stream;
use crate::unit::Unit;
use crate::Proxy;

use url::Url;

const DEFAULT_MAX_IDLE_CONNECTIONS: usize = 100;
const DEFAULT_MAX_IDLE_CONNECTIONS_PER_HOST: usize = 1;

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
#[derive(Debug)]
pub(crate) struct ConnectionPool {
    // the actual pooled connection. however only one per hostname:port.
    recycle: HashMap<PoolKey, VecDeque<Stream>>,
    // This is used to keep track of which streams to expire when the
    // pool reaches MAX_IDLE_CONNECTIONS. The corresponding PoolKeys for
    // recently used Streams are added to the back of the queue;
    // old streams are removed from the front.
    lru: VecDeque<PoolKey>,
    max_idle_connections: usize,
    max_idle_connections_per_host: usize,
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

impl Default for ConnectionPool {
    fn default() -> Self {
        Self {
            max_idle_connections: DEFAULT_MAX_IDLE_CONNECTIONS,
            max_idle_connections_per_host: DEFAULT_MAX_IDLE_CONNECTIONS_PER_HOST,
            recycle: HashMap::default(),
            lru: VecDeque::default(),
        }
    }
}

impl ConnectionPool {
    pub fn set_max_idle_connections(&mut self, max_connections: usize) {
        if self.max_idle_connections == max_connections {
            return;
        }

        // Remove any extra connections if the number was decreased.
        while self.lru.len() > max_connections {
            self.remove_oldest();
        }
        self.max_idle_connections = max_connections;
    }

    /// Return true if either of the max_* settings is 0, meaning we should do no work.
    fn noop(&self) -> bool {
        self.max_idle_connections == 0 || self.max_idle_connections_per_host == 0
    }

    pub fn set_max_idle_connections_per_host(&mut self, max_connections: usize) {
        if self.max_idle_connections_per_host == max_connections {
            return;
        }

        if max_connections == 0 {
            // Clear the connection pool, caching is disabled.
            self.lru.clear();
            self.recycle.clear();
            return;
        }

        // Remove any extra streams if the number was decreased.
        for (key, val) in self.recycle.iter_mut() {
            while val.len() > max_connections {
                // Remove the oldest entry
                val.pop_front();
                remove_first_match(&mut self.lru, key)
                    .expect("invariant failed: key in recycle but not in lru");
            }
        }
        self.max_idle_connections_per_host = max_connections;
    }

    /// How the unit::connect tries to get a pooled connection.
    pub fn try_get_connection(&mut self, url: &Url, proxy: &Option<Proxy>) -> Option<Stream> {
        let key = PoolKey::new(url, proxy);
        self.remove(&key)
    }

    fn remove(&mut self, key: &PoolKey) -> Option<Stream> {
        match self.recycle.entry(key.clone()) {
            Entry::Occupied(mut occupied_entry) => {
                let streams = occupied_entry.get_mut();
                // Take the newest stream.
                let stream = streams.pop_back();
                let stream = stream.expect("invariant failed: empty VecDeque in `recycle`");

                if streams.len() == 0 {
                    occupied_entry.remove();
                }

                // Remove the newest matching PoolKey from self.lru. That
                // corresponds to the stream we just removed from `recycle`.
                remove_last_match(&mut self.lru, &key)
                    .expect("invariant failed: key in recycle but not in lru");

                Some(stream)
            }
            Entry::Vacant(_) => None,
        }
    }

    fn add(&mut self, key: PoolKey, stream: Stream) {
        if self.noop() {
            return;
        }

        match self.recycle.entry(key.clone()) {
            Entry::Occupied(mut occupied_entry) => {
                let streams = occupied_entry.get_mut();
                streams.push_back(stream);
                if streams.len() > self.max_idle_connections_per_host {
                    // Remove the oldest entry
                    streams.pop_front();
                    remove_first_match(&mut self.lru, &key)
                        .expect("invariant failed: key in recycle but not in lru");
                }
            }
            Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(vec![stream].into());
            }
        }
        self.lru.push_back(key);
        if self.lru.len() > self.max_idle_connections {
            self.remove_oldest()
        }
    }

    /// Find the oldest stream in the pool. Remove its representation from lru,
    /// and the stream itself from `recycle`. Drops the stream, which closes it.
    fn remove_oldest(&mut self) {
        assert!(!self.noop(), "remove_oldest called on Pool with max of 0");
        let key = self.lru.pop_front();
        let key = key.expect("tried to remove oldest but no entries found!");
        match self.recycle.entry(key) {
            Entry::Occupied(mut occupied_entry) => {
                let streams = occupied_entry.get_mut();
                streams
                    .pop_front()
                    .expect("invariant failed: key existed in recycle but no streams available");
                if streams.len() == 0 {
                    occupied_entry.remove();
                }
            }
            Entry::Vacant(_) => panic!("invariant failed: key existed in lru but not in recycle"),
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lru.len()
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
    fn new(url: &Url, proxy: &Option<Proxy>) -> Self {
        let port = url.port_or_known_default();
        PoolKey {
            scheme: url.scheme().to_string(),
            hostname: url.host_str().unwrap_or("").to_string(),
            port,
            proxy: proxy.clone(),
        }
    }
}

#[test]
fn poolkey_new() {
    // Test that PoolKey::new() does not panic on unrecognized schemes.
    PoolKey::new(&Url::parse("zzz:///example.com").unwrap(), &None);
}

#[test]
fn pool_connections_limit() {
    // Test inserting connections with different keys into the pool,
    // filling and draining it. The pool should evict earlier connections
    // when the connection limit is reached.
    let mut pool = ConnectionPool::default();
    let hostnames = (0..DEFAULT_MAX_IDLE_CONNECTIONS * 2).map(|i| format!("{}.example", i));
    let poolkeys = hostnames.map(|hostname| PoolKey {
        scheme: "https".to_string(),
        hostname,
        port: Some(999),
        proxy: None,
    });
    for key in poolkeys.clone() {
        pool.add(key, Stream::Cursor(std::io::Cursor::new(vec![])));
    }
    assert_eq!(pool.len(), DEFAULT_MAX_IDLE_CONNECTIONS);

    for key in poolkeys.skip(DEFAULT_MAX_IDLE_CONNECTIONS) {
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
    let mut pool = ConnectionPool::default();
    let poolkey = PoolKey {
        scheme: "https".to_string(),
        hostname: "example.com".to_string(),
        port: Some(999),
        proxy: None,
    };

    for _ in 0..pool.max_idle_connections_per_host * 2 {
        pool.add(
            poolkey.clone(),
            Stream::Cursor(std::io::Cursor::new(vec![])),
        );
    }
    assert_eq!(pool.len(), DEFAULT_MAX_IDLE_CONNECTIONS_PER_HOST);

    for _ in 0..DEFAULT_MAX_IDLE_CONNECTIONS_PER_HOST {
        let result = pool.remove(&poolkey);
        assert!(result.is_some(), "expected key was not in pool");
    }
    assert_eq!(pool.len(), 0);
}

#[test]
fn pool_update_connection_limit() {
    let mut pool = ConnectionPool::default();
    pool.set_max_idle_connections(50);

    let hostnames = (0..pool.max_idle_connections).map(|i| format!("{}.example", i));
    let poolkeys = hostnames.map(|hostname| PoolKey {
        scheme: "https".to_string(),
        hostname,
        port: Some(999),
        proxy: None,
    });
    for key in poolkeys.clone() {
        pool.add(key, Stream::Cursor(std::io::Cursor::new(vec![])));
    }
    assert_eq!(pool.len(), 50);
    pool.set_max_idle_connections(25);
    assert_eq!(pool.len(), 25);
}

#[test]
fn pool_update_per_host_connection_limit() {
    let mut pool = ConnectionPool::default();
    pool.set_max_idle_connections(50);
    pool.set_max_idle_connections_per_host(50);

    let poolkey = PoolKey {
        scheme: "https".to_string(),
        hostname: "example.com".to_string(),
        port: Some(999),
        proxy: None,
    };

    for _ in 0..50 {
        pool.add(
            poolkey.clone(),
            Stream::Cursor(std::io::Cursor::new(vec![])),
        );
    }
    assert_eq!(pool.len(), 50);
    pool.set_max_idle_connections_per_host(25);
    assert_eq!(pool.len(), 25);
}

#[test]
fn pool_checks_proxy() {
    // Test inserting different poolkeys with same address but different proxies.
    // Each insertion should result in an additional entry in the pool.
    let mut pool = ConnectionPool::default();
    let url = Url::parse("zzz:///example.com").unwrap();

    pool.add(
        PoolKey::new(&url, &None),
        Stream::Cursor(std::io::Cursor::new(vec![])),
    );
    assert_eq!(pool.len(), 1);

    pool.add(
        PoolKey::new(&url, &Some(Proxy::new("localhost:9999").unwrap())),
        Stream::Cursor(std::io::Cursor::new(vec![])),
    );
    assert_eq!(pool.len(), 2);

    pool.add(
        PoolKey::new(
            &url,
            &Some(Proxy::new("user:password@localhost:9999").unwrap()),
        ),
        Stream::Cursor(std::io::Cursor::new(vec![])),
    );
    assert_eq!(pool.len(), 3);
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

    fn return_connection(&mut self) -> io::Result<()> {
        // guard we only do this once.
        if let (Some(unit), Some(reader)) = (self.unit.take(), self.reader.take()) {
            let state = &mut unit.req.agent.lock().unwrap();
            // bring back stream here to either go into pool or dealloc
            let mut stream = reader.into();
            if !stream.is_poolable() {
                // just let it deallocate
                return Ok(());
            }

            // ensure stream can be reused
            stream.reset()?;

            // insert back into pool
            let key = PoolKey::new(&unit.url, &unit.req.proxy);
            state.pool().add(key, stream);
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
