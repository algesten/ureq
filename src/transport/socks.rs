use core::fmt;
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;

use socks::{Socks4Stream, Socks5Stream};

use crate::error::TimeoutReason;
use crate::proxy::{Proto, Proxy};
use crate::transport::tcp::TcpTransport;
use crate::transport::LazyBuffers;
use crate::Error;

use super::{ConnectionDetails, Connector, Transport};

#[derive(Default)]
pub struct SocksConnector {}

impl Connector for SocksConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        let proxy = match &details.proxy {
            Some(v) if v.proto().is_socks() => v,
            // If there is no proxy configured, or it isn't a SOCKS proxy, use whatever is chained.
            _ => {
                trace!("SOCKS not configured");
                return Ok(chained);
            }
        };

        if chained.is_some() {
            trace!("Skip");
            return Ok(chained);
        }

        trace!("Try connect SOCKS {} -> {}", proxy.uri(), details.addr);

        let proxy_addr = details.resolver.resolve(proxy.uri(), details.timeout)?;
        let target_addr = details.addr;

        // The async behavior is only used if we want to time cap connecting.
        let use_sync = details.timeout.is_not_happening();

        let stream = if use_sync {
            connect_proxy(proxy, proxy_addr, target_addr)?
        } else {
            let (tx, rx) = mpsc::sync_channel(1);
            let proxy = proxy.clone();

            thread::spawn(move || tx.send(connect_proxy(&proxy, proxy_addr, target_addr)));

            match rx.recv_timeout(*details.timeout) {
                Ok(v) => v?,
                Err(RecvTimeoutError::Timeout) => return Err(Error::Timeout(TimeoutReason::Socks)),
                Err(RecvTimeoutError::Disconnected) => unreachable!("mpsc sender gone"),
            }
        };

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size,
            details.config.output_buffer_size,
        );
        let transport = Box::new(TcpTransport::new(stream, buffers));

        debug!("SOCKS connected {} -> {}", proxy.uri(), details.addr);

        Ok(Some(transport))
    }
}

fn connect_proxy(
    proxy: &Proxy,
    proxy_addr: SocketAddr,
    target_addr: SocketAddr,
) -> Result<TcpStream, Error> {
    let stream = match proxy.proto() {
        Proto::SOCKS4 | Proto::SOCKS4A => {
            if proxy.username().is_some() {
                warn!("SOCKS4 does not support username/password");
            }

            Socks4Stream::connect(proxy_addr, target_addr, "")?.into_inner()
        }
        Proto::SOCKS5 => {
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
