use std::fmt;
use std::iter::once;
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::{io, thread};

use socks::{Socks4Stream, Socks5Stream, ToTargetAddr};

use crate::proxy::{Proxy, ProxyProtocol};
use crate::util::UriExt;
use crate::Error;

use super::chain::Either;
use super::ResolvedSocketAddrs;

use super::tcp::TcpTransport;
use super::{ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport};

/// Connector for SOCKS proxies.
///
/// Requires the **socks-proxy** feature.
///
/// The connector looks at the proxy settings in [`proxy`](crate::config::ConfigBuilder::proxy) to
/// determine whether to attempt a proxy connection or not.
#[derive(Default)]
pub struct SocksConnector(());

impl<In: Transport> Connector<In> for SocksConnector {
    type Out = Either<In, TcpTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        let proxy = match details.config.proxy() {
            Some(v) if v.protocol().is_socks() => v,
            // If there is no proxy configured, or it isn't a SOCKS proxy, use whatever is chained.
            _ => {
                trace!("SOCKS not configured");
                return Ok(chained.map(Either::A));
            }
        };

        if chained.is_some() {
            trace!("Skip");
            return Ok(chained.map(Either::A));
        }

        let proxy_addrs = details
            .resolver
            .resolve(proxy.uri(), details.config, details.timeout)?;

        // Check if this host is not supposed to be proxied.
        let is_no_proxy = details
            .config
            .proxy()
            .map(|p| p.is_no_proxy(details.uri))
            .unwrap_or(false);

        if is_no_proxy {
            return Ok(None);
        }

        let stream = if proxy.resolve_target() {
            // The target is already resolved by run().
            let resolved = details.addrs.iter().cloned();

            try_connect(&proxy_addrs, resolved, proxy, details.timeout)?
        } else {
            // Do not to resolve the target locally, instead pass (host, port)
            // to the proxy and let it resolve.
            let iter = once(details.uri.host_port());
            try_connect(&proxy_addrs, iter, proxy, details.timeout)?
        };

        if details.config.no_delay() {
            stream.set_nodelay(true)?;
        }

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );
        let transport = TcpTransport::new(stream, buffers);

        Ok(Some(Either::B(transport)))
    }
}

fn try_connect<'a, T: ToTargetAddr + fmt::Debug + Send + 'a + Clone>(
    proxy_addrs: &ResolvedSocketAddrs,
    target_addrs: impl Iterator<Item = T>,
    proxy: &Proxy,
    timeout: NextTimeout,
) -> Result<TcpStream, Error> {
    for target_addr in target_addrs {
        for proxy_addr in proxy_addrs {
            trace!(
                "Try connect {} {} -> {:?}",
                proxy.protocol(),
                proxy_addr,
                target_addr
            );

            match try_connect_single(*proxy_addr, target_addr.clone(), proxy, timeout) {
                Ok(v) => {
                    debug!(
                        "{} connected {} -> {:?}",
                        proxy.protocol(),
                        proxy_addr,
                        target_addr
                    );
                    return Ok(v);
                }
                // Intercept ConnectionRefused to try next addrs
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::ConnectionRefused => {
                    trace!(
                        "{} -> {:?} proxy connection refused",
                        proxy_addr,
                        target_addr
                    );
                    continue;
                }
                // Other errors bail
                Err(e) => return Err(e),
            }
        }
    }

    debug!("Proxy failed to to connect to any resolved address");
    Err(Error::Io(io::Error::new(
        io::ErrorKind::ConnectionRefused,
        "Connection refused",
    )))
}

fn try_connect_single<'a, T: ToTargetAddr + Send + 'a>(
    proxy_addr: SocketAddr,
    target_addr: T,
    proxy: &Proxy,
    timeout: NextTimeout,
) -> Result<TcpStream, Error> {
    // The async behavior is only used if we want to time cap connecting.
    let use_sync = timeout.after.is_not_happening();

    if use_sync {
        connect_proxy(proxy, proxy_addr, target_addr)
    } else {
        let (tx, rx) = mpsc::sync_channel(1);
        let proxy = proxy.clone();

        thread::scope(move |s| {
            s.spawn(move || tx.send(connect_proxy(&proxy, proxy_addr, target_addr)));

            match rx.recv_timeout(*timeout.after) {
                Ok(v) => v,
                Err(RecvTimeoutError::Timeout) => Err(Error::Timeout(timeout.reason)),
                Err(RecvTimeoutError::Disconnected) => unreachable!("mpsc sender gone"),
            }
        })
    }
}

fn connect_proxy<'a, T: ToTargetAddr + 'a>(
    proxy: &Proxy,
    proxy_addr: SocketAddr,
    target_addr: T,
) -> Result<TcpStream, Error> {
    let stream = match proxy.protocol() {
        ProxyProtocol::Socks4 | ProxyProtocol::Socks4A => {
            if proxy.username().is_some() {
                debug!("SOCKS4 does not support username/password");
            }

            Socks4Stream::connect(proxy_addr, target_addr, "")?.into_inner()
        }

        ProxyProtocol::Socks5 | ProxyProtocol::Socks5h => {
            if let Some(username) = proxy.username() {
                // Connect with authentication.
                let password = proxy.password().unwrap_or("");

                Socks5Stream::connect_with_password(proxy_addr, target_addr, username, password)?
            } else {
                Socks5Stream::connect(proxy_addr, target_addr)?
            }
            .into_inner()
        }

        _ => unreachable!(), // HTTP(s) proxies.
    };

    Ok(stream)
}

impl fmt::Debug for SocksConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SocksConnector").finish()
    }
}
