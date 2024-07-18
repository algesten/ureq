use std::fmt::{self, Debug};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self};
use std::vec::IntoIter;

use http::uri::{Authority, Scheme};
use http::Uri;

use crate::time::NextTimeout;
use crate::util::SchemeExt;
use crate::Error;

pub trait Resolver: Debug + Send + Sync + 'static {
    fn resolve(&self, uri: &Uri, timeout: NextTimeout) -> Result<SocketAddr, Error>;
}

pub struct DefaultResolver {
    family: IpFamily,
    select: AddrSelect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpFamily {
    Any,
    Ipv4Only,
    Ipv6Only,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AddrSelect {
    First,
    // TODO(martin): implement round robin per hostname
}
impl DefaultResolver {
    pub fn host_and_port(scheme: &Scheme, authority: &Authority) -> String {
        let port = authority
            .port_u16()
            .unwrap_or_else(|| scheme.default_port());

        format!("{}:{}", authority.host(), port)
    }
}

impl Resolver for DefaultResolver {
    fn resolve(&self, uri: &Uri, timeout: NextTimeout) -> Result<SocketAddr, Error> {
        let scheme = uri.scheme().ok_or(Error::Other("No scheme in uri"))?;
        let authority = uri.authority().ok_or(Error::Other("No host in uri"))?;

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
