use std::collections::{HashMap, VecDeque};
use std::io::{Read, Result as IoResult};

use crate::stream::Stream;
use crate::unit::Unit;
use crate::Proxy;

use url::Url;

pub const DEFAULT_HOST: &str = "localhost";
const MAX_IDLE_CONNECTIONS: usize = 100;

/// Holder of recycled connections.
///
/// Invariant: The length of recycle and lru are the same.
/// Invariant: Each PoolKey exists as a key in recycle, and vice versa.
/// Invariant: Each PoolKey exists in recycle at most once and lru at most once.
///
/// *Internal API*
#[derive(Default, Debug)]
pub(crate) struct ConnectionPool {
    // the actual pooled connection. however only one per hostname:port.
    recycle: HashMap<PoolKey, Stream>,
    // This is used to keep track of which streams to expire when the
    // pool reaches MAX_IDLE_CONNECTIONS. The corresponding PoolKeys for
    // recently used Streams are added to the back of the queue;
    // old streams are removed from the front.
    lru: VecDeque<PoolKey>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        ConnectionPool {
            ..Default::default()
        }
    }

    /// How the unit::connect tries to get a pooled connection.
    pub fn try_get_connection(&mut self, url: &Url, proxy: &Option<Proxy>) -> Option<Stream> {
        let key = PoolKey::new(url, proxy);
        self.remove(&key)
    }

    fn remove(&mut self, key: &PoolKey) -> Option<Stream> {
        if !self.recycle.contains_key(&key) {
            return None;
        }
        let index = self.lru.iter().position(|k| k == key);
        assert!(
            index.is_some(),
            "invariant failed: key existed in recycle but not lru"
        );
        self.lru.remove(index.unwrap());
        self.recycle.remove(&key)
    }

    fn add(&mut self, key: PoolKey, stream: Stream) {
        // If an entry with the same key already exists, remove it.
        // The more recently used stream is likely to live longer.
        self.remove(&key);
        if self.recycle.len() + 1 > MAX_IDLE_CONNECTIONS {
            self.remove_oldest();
        }
        self.lru.push_back(key.clone());
        self.recycle.insert(key, stream);
    }

    fn remove_oldest(&mut self) {
        if let Some(key) = self.lru.pop_front() {
            let removed = self.recycle.remove(&key);
            assert!(
                removed.is_some(),
                "invariant failed: key existed in lru but not in recycle"
            );
        } else {
            panic!("tried to remove oldest but no entries found!");
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.recycle.len()
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
fn pool_size_limit() {
    assert_eq!(MAX_IDLE_CONNECTIONS, 100);
    let mut pool = ConnectionPool::new();
    let hostnames = (0..200).map(|i| format!("{}.example", i));
    let poolkeys = hostnames.map(|hostname| PoolKey {
        scheme: "https".to_string(),
        hostname,
        port: Some(999),
        proxy: None,
    });
    for key in poolkeys.clone() {
        pool.add(key, Stream::Cursor(std::io::Cursor::new(vec![])));
    }
    assert_eq!(pool.len(), 100);

    for key in poolkeys.skip(100) {
        let result = pool.remove(&key);
        assert!(result.is_some(), "expected key was not in pool");
    }
}

#[test]
fn pool_duplicates_limit() {
    // Test inserting duplicates into the pool, and subsequently
    // filling and draining it. The duplicates should evict earlier
    // entries with the same key.
    assert_eq!(MAX_IDLE_CONNECTIONS, 100);
    let mut pool = ConnectionPool::new();
    let hostnames = (0..100).map(|i| format!("{}.example", i));
    let poolkeys = hostnames.map(|hostname| PoolKey {
        scheme: "https".to_string(),
        hostname,
        port: Some(999),
        proxy: None,
    });
    for key in poolkeys.clone() {
        pool.add(key.clone(), Stream::Cursor(std::io::Cursor::new(vec![])));
        pool.add(key, Stream::Cursor(std::io::Cursor::new(vec![])));
    }
    assert_eq!(pool.len(), 100);

    for key in poolkeys {
        let result = pool.remove(&key);
        assert!(result.is_some(), "expected key was not in pool");
    }
}

#[test]
fn pool_checks_proxy() {
    // Test inserting different poolkeys with same address but different proxies.
    // Each insertion should result in an additional entry in the pool.
    let mut pool = ConnectionPool::new();
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
                let key = PoolKey::new(&unit.url, &unit.proxy);
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
