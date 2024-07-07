use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self};
use std::time::Duration;

use http::Uri;

use crate::error::TimeoutReason;
use crate::Error;

pub trait Resolver: fmt::Debug + 'static {
    fn resolve(&self, uri: &Uri, timeout: Duration) -> Result<SocketAddr, Error>;
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
pub enum AddrSelect {
    First,
    // TODO(martin): implement round robin per hostname
}

impl Resolver for DefaultResolver {
    fn resolve(&self, uri: &Uri, timeout: Duration) -> Result<SocketAddr, Error> {
        let host = uri
            .authority()
            .map(|a| a.host())
            .ok_or(Error::Other("No host in uri"))?
            // There is no way around allocating here. We can't use a scoped thread below,
            // because if we time out, we're going to exit the function and leave the
            // thread running (causing a panic as per scoped thread contract).
            .to_string();

        // TODO(martin): On Linux we have getaddrinfo_a which is a libc async way of
        // doing host lookup. We should make a subcrate that uses a native async method
        // when possible, and otherwise fall back on this thread behavior.
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || tx.send(host.to_socket_addrs()).ok());

        let iter = match rx.recv_timeout(timeout) {
            Ok(v) => v,
            Err(c) => match c {
                // Timeout results in None
                RecvTimeoutError::Timeout => return Err(Error::Timeout(TimeoutReason::Resolver)),
                // The sender going away is nonsensical. Did the thread just die?
                RecvTimeoutError::Disconnected => unreachable!("mpsc sender gone"),
            },
        }?;

        let wanted = self.family.keep_wanted(iter);
        let maybe_addr = self.select.choose(wanted);

        maybe_addr.ok_or(Error::HostNotFound)
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
