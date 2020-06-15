use std::collections::HashMap;
use std::io::{Read, Result as IoResult};

use crate::stream::Stream;
use crate::unit::Unit;

use url::Url;

pub const DEFAULT_HOST: &str = "localhost";

/// Holder of recycled connections.
///
/// *Internal API*
#[derive(Default, Debug)]
pub(crate) struct ConnectionPool {
    // the actual pooled connection. however only one per hostname:port.
    recycle: HashMap<PoolKey, Stream>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        ConnectionPool {
            ..Default::default()
        }
    }

    /// How the unit::connect tries to get a pooled connection.
    pub fn try_get_connection(&mut self, url: &Url) -> Option<Stream> {
        self.recycle.remove(&PoolKey::new(url))
    }

    #[cfg(all(test, any(feature = "tls", feature = "native-tls")))]
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
pub(crate) struct PoolReturnRead<R: Read + Sized> {
    // unit that contains the agent where we want to return the reader.
    unit: Option<Unit>,
    // pointer to underlying stream.
    // this pointer forces the entire PoolReturnRead to be !Sync and !Send
    // that's a good thing, because the pool return logic is certainly not
    // thread safe.
    stream: *mut Stream,
    // wrapped reader around the same stream
    reader: Option<R>,
}

impl<R: Read + Sized> PoolReturnRead<R> {
    pub fn new(unit: Option<Unit>, stream: *mut Stream, reader: R) -> Self {
        PoolReturnRead {
            unit,
            stream,
            reader: Some(reader),
        }
    }

    fn return_connection(&mut self) {
        // guard we only do this once.
        if let Some(unit) = self.unit.take() {
            // this frees up the wrapper type around the Stream so
            // we can safely bring the stream pointer back.
            self.reader.take();
            if self.stream.is_null() {
                return;
            }
            let state = &mut unit.agent.lock().unwrap();
            // bring back stream here to either go into pool or dealloc
            let stream = unsafe { *Box::from_raw(self.stream) };
            self.stream = ::std::ptr::null_mut();
            if let Some(agent) = state.as_mut() {
                if !stream.is_poolable() {
                    // just let it deallocate
                    return;
                }
                // insert back into pool
                let key = PoolKey::new(&unit.url);
                agent.pool().recycle.insert(key, stream);
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

impl<R: Read + Sized> Read for PoolReturnRead<R> {
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

impl<R: Read + Sized> Drop for PoolReturnRead<R> {
    fn drop(&mut self) {
        self.return_connection();
    }
}
