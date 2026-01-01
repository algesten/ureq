use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use std::borrow::Cow;
use std::fmt;
use std::io::Write;
use ureq_proto::parser::try_parse_response;

use http::StatusCode;

use crate::config::DEFAULT_USER_AGENT;
use crate::http;
use crate::transport::{ConnectionDetails, Connector, Either, Transport, TransportAdapter};
use crate::util::{SchemeExt, UriExt};
use crate::Error;

/// Connector for CONNECT proxy settings.
///
/// This operates on the previous chained transport typically a TcpConnector optionally
/// wrapped in TLS.
#[derive(Default)]
pub struct ConnectProxyConnector(());

impl<In: Transport> Connector<In> for ConnectProxyConnector {
    type Out = Either<In, Box<dyn Transport>>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        // If there is already a connection, do nothing.
        if let Some(transport) = chained {
            return Ok(Some(Either::A(transport)));
        }

        // If we're using a CONNECT proxy, we need to resolve that hostname.
        let maybe_connect_uri = details.config.connect_proxy_uri();

        let Some(connect_uri) = maybe_connect_uri else {
            // Not using CONNECT
            return Ok(None);
        };

        let target = details.uri;
        let target_addrs = &details.addrs;

        // Check if this host matches NO_PROXY
        let is_no_proxy = details
            .config
            .proxy()
            .map(|p| p.is_no_proxy(target))
            .unwrap_or(false);

        if is_no_proxy {
            return Ok(None);
        }

        // TODO(martin): it's a bit weird to put the CONNECT proxy
        // resolver timeout as part of Timeout::Connect, but we don't
        // have anything more granular for now.
        let proxy_addrs = details
            .resolver
            .resolve(connect_uri, details.config, details.timeout)?;

        let proxy_config = details.config.clone_without_proxy();

        // ConnectionDetails to establish a connection to the CONNECT
        // proxy itself.
        let proxy_details = ConnectionDetails {
            uri: connect_uri,
            addrs: proxy_addrs,
            config: &proxy_config,
            request_level: details.request_level,
            resolver: details.resolver,
            now: details.now,
            timeout: details.timeout,
            current_time: details.current_time.clone(),
            run_connector: details.run_connector.clone(),
        };

        let transport = (details.run_connector)(&proxy_details)?;

        // unwrap is ok because connect_proxy_uri() above checks it.
        let proxy = details.config.proxy().unwrap();

        let mut w = TransportAdapter::new(transport);

        target.ensure_valid_url()?;

        // unwraps are ok because ensure_valid_url() checks it.
        let mut target_host = Cow::Borrowed(target.host().unwrap());
        let target_port = target
            .port_u16()
            .unwrap_or(target.scheme().unwrap().default_port().unwrap());

        if proxy.resolve_target() {
            // In run() we do the resolution of the target (proxied) host, at this
            // point we should have at least one IP address.
            //
            // TODO(martin): On fail try more addresses
            let resolved = target_addrs.first().expect("at least one resolved address");

            target_host = Cow::Owned(resolved.to_string());
        }

        write!(w, "CONNECT {}:{} HTTP/1.1\r\n", target_host, target_port)?;
        write!(w, "Host: {}:{}\r\n", target_host, target_port)?;
        if let Some(v) = details.config.user_agent().as_str(DEFAULT_USER_AGENT) {
            write!(w, "User-Agent: {}\r\n", v)?;
        }
        write!(w, "Proxy-Connection: Keep-Alive\r\n")?;

        let use_creds = proxy.username().is_some() || proxy.password().is_some();

        if use_creds {
            let user = proxy.username().unwrap_or_default();
            let pass = proxy.password().unwrap_or_default();
            let creds = BASE64_STANDARD.encode(format!("{}:{}", user, pass));
            write!(w, "Proxy-Authorization: basic {}\r\n", creds)?;
        }

        write!(w, "\r\n")?;
        w.flush()?;

        let mut transport = w.into_inner();

        let response = loop {
            let made_progress = transport.maybe_await_input(details.timeout)?;
            let buffers = transport.buffers();
            let input = buffers.input();
            let Some((used_input, response)) = try_parse_response::<20>(input)? else {
                if !made_progress {
                    let reason = "proxy server did not respond".to_string();
                    return Err(Error::ConnectProxyFailed(reason));
                }
                continue;
            };
            buffers.input_consume(used_input);
            break response;
        };

        match response.status() {
            StatusCode::OK => {
                trace!("CONNECT proxy connected");
            }
            x => {
                let reason = format!("proxy server responded {}/{}", x.as_u16(), x.as_str());
                return Err(Error::ConnectProxyFailed(reason));
            }
        }

        Ok(Some(Either::B(transport)))
    }
}

impl fmt::Debug for ConnectProxyConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyConnector").finish()
    }
}
