//! Name resolvers.
//!
//! **NOTE resolver does not (yet) [follow semver][super].**
//!
//! _NOTE: Resolver is deep configuration of ureq and is not required for regular use._
//!
//! Name resolving is pluggable. The resolver's duty is to take a URI and translate it
//! to a socket address (IP + port). This is done as a separate step in regular ureq use.
//! The hostname is looked up and provided to the [`Connector`](crate::transport::Connector).
//!
//! In some situations it might be desirable to not do this lookup, or to use another system
//! than DNS for it.
use std::fmt::{self, Debug};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self};
use std::vec::IntoIter;

use http::uri::{Authority, Scheme};
use http::Uri;

use crate::config::Config;
use crate::http;
use crate::transport::NextTimeout;
use crate::util::{SchemeExt, UriExt};
use crate::Error;

/// Trait for name resolvers.
pub trait Resolver: Debug + Send + Sync + 'static {
    /// Resolve the URI to a socket address.
    ///
    /// The implementation should resolve within the given _timeout_.
    ///
    /// The resolver must guarantee at least one returned address, or error with
    /// `Error::HostNotFound`.
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error>;

    /// Produce an empty array of addresses.
    fn empty(&self) -> ResolvedSocketAddrs {
        fn uninited_socketaddr() -> SocketAddr {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0)
        }

        ArrayVec::from_fn(|_| uninited_socketaddr())
    }
}

/// Max number of socket addresses to keep from the resolver.
const MAX_ADDRS: usize = 16;

pub use ureq_proto::ArrayVec;

/// Addresses as returned by the resolver.
pub type ResolvedSocketAddrs = ArrayVec<SocketAddr, MAX_ADDRS>;

/// Default resolver implementation.
///
/// Uses std::net [`ToSocketAddrs`](https://doc.rust-lang.org/std/net/trait.ToSocketAddrs.html) to
/// do the lookup. Can optionally spawn a thread to abort lookup if the relevant timeout is set.
#[derive(Default)]
pub struct DefaultResolver {
    _private: (),
}

impl DefaultResolver {
    /// Helper to combine scheme host and port to a single string.
    ///
    /// This knows about the default ports for http, https and socks proxies which
    /// can then be omitted from the `Authority`.
    pub fn host_and_port(scheme: &Scheme, authority: &Authority) -> Option<String> {
        let port = authority.port_u16().or_else(|| scheme.default_port())?;

        Some(format!("{}:{}", authority.host(), port))
    }
}

impl Resolver for DefaultResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        uri.ensure_valid_url()?;

        // unwrap is ok due to ensure_full_url() above.
        let scheme = uri.scheme().unwrap();
        let authority = uri.authority().unwrap();

        if cfg!(feature = "_test") {
            let mut v = ArrayVec::from_fn(|_| "0.0.0.0:1".parse().unwrap());
            v.push(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::new(10, 0, 0, 1),
                authority
                    .port_u16()
                    .or_else(|| scheme.default_port())
                    // unwrap is ok because ensure_valid_url() above.
                    .unwrap(),
            )));
            return Ok(v);
        }

        // This will be on the form "myspecialhost.org:1234". The port is mandatory.
        // unwrap is ok because ensure_valid_url() above.
        let addr = DefaultResolver::host_and_port(scheme, authority).unwrap();

        // Determine if we want to use the async behavior.
        let use_sync = timeout.after.is_not_happening();

        let iter = if use_sync {
            trace!("Resolve: {}", addr);
            // When timeout is not set, we do not spawn any threads.
            addr.to_socket_addrs()?
        } else {
            trace!("Resolve with timeout ({:?}): {} ", timeout, addr);
            resolve_async(addr, timeout)?
        };

        let ip_family = config.ip_family();
        let wanted = ip_family.keep_wanted(iter);

        let mut result = self.empty();
        for addr in wanted.take(MAX_ADDRS) {
            result.push(addr);
        }

        debug!("Resolved: {:?}", result);

        if result.is_empty() {
            Err(Error::HostNotFound)
        } else {
            Ok(result)
        }
    }
}

fn resolve_async(addr: String, timeout: NextTimeout) -> Result<IntoIter<SocketAddr>, Error> {
    // TODO(martin): On Linux we have getaddrinfo_a which is a libc async way of
    // doing host lookup. We should make a subcrate that uses a native async method
    // when possible, and otherwise fall back on this thread behavior.
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || tx.send(addr.to_socket_addrs()).ok());

    match rx.recv_timeout(*timeout.after) {
        Ok(v) => Ok(v?),
        Err(c) => match c {
            // Timeout results in None
            RecvTimeoutError::Timeout => Err(Error::Timeout(timeout.reason)),
            // The sender going away is nonsensical. Did the thread just die?
            RecvTimeoutError::Disconnected => unreachable!("mpsc sender gone"),
        },
    }
}

impl fmt::Debug for DefaultResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefaultResolver").finish()
    }
}

#[cfg(test)]
mod test {
    use crate::transport::time::Duration;

    use super::*;

    #[test]
    fn unknown_scheme() {
        let uri: Uri = "foo://some:42/123".parse().unwrap();
        let config = Config::default();
        let err = DefaultResolver::default()
            .resolve(
                &uri,
                &config,
                NextTimeout {
                    after: Duration::NotHappening,
                    reason: crate::Timeout::Global,
                },
            )
            .unwrap_err();
        assert!(matches!(err, Error::BadUri(_)));
        assert_eq!(err.to_string(), "bad uri: unknown scheme: foo");
    }
}
