//! Name resolvers.
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
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self};
use std::vec::IntoIter;

use http::uri::{Authority, Scheme};
use http::Uri;

use crate::transport::time::NextTimeout;
use crate::util::{SchemeExt, UriExt};
use crate::Error;

/// Trait for name resolvers.
pub trait Resolver: Debug + Send + Sync + 'static {
    fn resolve(&self, uri: &Uri, timeout: NextTimeout) -> Result<SocketAddr, Error>;
}

/// Default resolver implementation.
///
/// Uses std::net [`ToSocketAddrs`](https://doc.rust-lang.org/std/net/trait.ToSocketAddrs.html) to
/// do the lookup. Can optionally spawn a thread to abort lookup if the relevant timeout is set.
pub struct DefaultResolver {
    family: IpFamily,
    select: AddrSelect,
}

/// Configuration of IP family to use.
///
/// Used to limit the IP to either IPv4, IPv6 or any.
// TODO(martin): make this configurable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpFamily {
    Any,
    Ipv4Only,
    Ipv6Only,
}

/// Strategy for selecting a single socket address.
///
/// A name server lookup might result in multiple socket addresses. This can happen for
/// multihomed servers or for crude load balancing.
///
/// This enumerates the implemented strategies for picking one address of all the returned ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AddrSelect {
    /// Pick first returned address.
    First,
    // TODO(martin): implement round robin per hostname and make it configurable
}

impl DefaultResolver {
    /// Helper to combine scheme host and port to a single string.
    ///
    /// This knows about the default ports for http, https and socks proxies which
    /// can then be omitted from the `Authority`.
    pub fn host_and_port(scheme: &Scheme, authority: &Authority) -> String {
        let port = authority
            .port_u16()
            .unwrap_or_else(|| scheme.default_port());

        format!("{}:{}", authority.host(), port)
    }
}

impl Resolver for DefaultResolver {
    fn resolve(&self, uri: &Uri, timeout: NextTimeout) -> Result<SocketAddr, Error> {
        uri.ensure_full_url()?;

        // unwrap is ok due to ensure_full_url() above.
        let scheme = uri.scheme().unwrap();
        let authority = uri.authority().unwrap();

        if cfg!(feature = "_test") {
            return Ok(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::new(10, 0, 0, 1),
                authority.port_u16().unwrap_or(scheme.default_port()),
            )));
        }

        // This will be on the form "myspecialhost.org:1234". The port is mandatory.
        let addr = DefaultResolver::host_and_port(scheme, authority);

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

        let wanted = self.family.keep_wanted(iter);
        let maybe_addr = self.select.choose(wanted);

        debug!("Resolved: {:?}", maybe_addr);

        maybe_addr.ok_or(Error::HostNotFound)
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

impl IpFamily {
    pub fn keep_wanted<'a>(
        &'a self,
        iter: impl Iterator<Item = SocketAddr> + 'a,
    ) -> impl Iterator<Item = SocketAddr> + 'a {
        iter.filter(move |a| self.is_wanted(a))
    }

    fn is_wanted(&self, addr: &SocketAddr) -> bool {
        match self {
            IpFamily::Any => true,
            IpFamily::Ipv4Only => addr.is_ipv4(),
            IpFamily::Ipv6Only => addr.is_ipv6(),
        }
    }
}

impl AddrSelect {
    pub fn choose(&self, mut iter: impl Iterator<Item = SocketAddr>) -> Option<SocketAddr> {
        match self {
            AddrSelect::First => iter.next(),
        }
    }
}

impl fmt::Debug for DefaultResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefaultResolver").finish()
    }
}

impl Default for DefaultResolver {
    fn default() -> Self {
        Self {
            family: IpFamily::Any,
            select: AddrSelect::First,
        }
    }
}
