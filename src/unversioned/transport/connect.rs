use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use std::borrow::Cow;
use std::fmt;
use std::io::Write;
use ureq_proto::parser::try_parse_response;

use http::StatusCode;

use crate::Error;
use crate::config::DEFAULT_USER_AGENT;
use crate::http;
use crate::transport::{Buffers, ConnectionDetails, Connector, Either};
use crate::transport::{NextTimeout, Transport, TransportAdapter};
use crate::util::{SchemeExt, UriExt};

use super::io::remaining_after;

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
        let timeout_at = |now| {
            let mut timeout = details.timeout;
            timeout.after = remaining_after(timeout.after, details.now, now);
            timeout.check()
        };
        let proxy_addrs = details.resolver.resolve(
            connect_uri,
            details.config,
            timeout_at((details.current_time)())?,
        )?;
        let now = (details.current_time)();

        let proxy_config = details.config.clone_without_proxy();

        // ConnectionDetails to establish a connection to the CONNECT
        // proxy itself.
        let proxy_details = ConnectionDetails {
            uri: connect_uri,
            addrs: proxy_addrs,
            config: &proxy_config,
            request_level: details.request_level,
            resolver: details.resolver,
            now,
            timeout: timeout_at(now)?,
            current_time: details.current_time.clone(),
            run_connector: details.run_connector.clone(),
            register_shutdown: details.register_shutdown.clone(),
        };

        let transport = (details.run_connector)(&proxy_details)?;

        // unwrap is ok because connect_proxy_uri() above checks it.
        let proxy = details.config.proxy().unwrap();

        let mut w = TransportAdapter::new(transport, details.current_time.clone());
        w.set_timeout_at(details.timeout, details.now);

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
            // "Basic" in canonical casing: the scheme is case-insensitive per
            // RFC 9110, but some proxies reject the lowercase form outright.
            write!(w, "Proxy-Authorization: Basic {}\r\n", creds)?;
        }

        write!(w, "\r\n")?;
        w.flush()?;

        let mut transport = w.into_inner();

        let response = loop {
            let made_progress =
                transport.maybe_await_input(timeout_at((details.current_time)())?)?;
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

        // CONNECT establishes a new connection context for the target. The transport
        // might have TLS to the proxy, but it does not have TLS to the target yet.
        Ok(Some(Either::B(TunnelTransport(transport).boxed())))
    }
}

#[derive(Debug)]
struct TunnelTransport(Box<dyn Transport>);

impl Transport for TunnelTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.0.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        self.0.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        self.0.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.0.is_open()
    }

    fn is_tls(&self) -> bool {
        false
    }
}

impl fmt::Debug for ConnectProxyConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyConnector").finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::Config;
    use crate::transport::LazyBuffers;
    use crate::transport::time::{Duration, Instant};
    use crate::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
    use crate::{Agent, Timeout};
    use std::collections::VecDeque;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct State {
        elapsed: u64,
        dns: Vec<NextTimeout>,
        connects: Vec<NextTimeout>,
        writes: Vec<NextTimeout>,
        reads: Vec<NextTimeout>,
    }

    #[derive(Debug)]
    struct TimedResolver(Arc<Mutex<State>>, u64);

    impl Resolver for TimedResolver {
        fn resolve(
            &self,
            _: &http::Uri,
            _: &Config,
            timeout: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, Error> {
            let mut state = self.0.lock().unwrap();
            state.dns.push(timeout);
            state.elapsed += self.1;
            let mut addrs = self.empty();
            addrs.push(SocketAddr::from(([127, 0, 0, 1], 8080)));
            Ok(addrs)
        }
    }

    #[derive(Debug)]
    struct TimedTransport {
        state: Arc<Mutex<State>>,
        replies: VecDeque<&'static [u8]>,
        buffers: LazyBuffers,
    }

    impl Transport for TimedTransport {
        fn buffers(&mut self) -> &mut dyn Buffers {
            &mut self.buffers
        }
        fn transmit_output(&mut self, _: usize, timeout: NextTimeout) -> Result<(), Error> {
            self.state
                .lock()
                .unwrap()
                .writes
                .push(timeout.for_write().check()?);
            Ok(())
        }
        fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
            let timeout = timeout.for_read().check()?;
            let mut state = self.state.lock().unwrap();
            state.reads.push(timeout);
            if timeout.after < Duration::from_millis(20) {
                state.elapsed += timeout.after.as_millis() as u64;
                return Err(Error::Timeout(timeout.reason));
            }
            state.elapsed += 20;
            let bytes = self.replies.pop_front().unwrap();
            self.buffers.input_append_buf()[..bytes.len()].copy_from_slice(bytes);
            self.buffers.input_appended(bytes.len());
            Ok(true)
        }
        fn is_open(&mut self) -> bool {
            true
        }
    }

    #[test]
    fn proxy_setup_and_fragmented_response_share_one_budget() {
        for reason in [Timeout::Global, Timeout::PerCall, Timeout::Connect] {
            for (budget, dns_time) in [(100, 20), (200, 20), (100, 100)] {
                let state = Arc::new(Mutex::new(State {
                    elapsed: 10,
                    ..State::default()
                }));
                let resolver = TimedResolver(state.clone(), dns_time);
                let config = Agent::config_builder()
                    .proxy(Some(crate::Proxy::new("http://127.0.0.1:8080").unwrap()))
                    .build();
                let uri = "http://example.test/".parse().unwrap();
                let start = Instant::now();
                let clock = state.clone();
                let connection_state = state.clone();
                let details = ConnectionDetails {
                    uri: &uri,
                    addrs: resolver.empty(),
                    config: &config,
                    request_level: false,
                    resolver: &resolver,
                    now: start,
                    timeout: NextTimeout {
                        after: Duration::from_millis(budget),
                        reason,
                        per_read: Some(Duration::from_secs(2)),
                        per_write: Some(Duration::from_secs(3)),
                    },
                    current_time: Arc::new(move || {
                        start + Duration::from_millis(clock.lock().unwrap().elapsed)
                    }),
                    run_connector: Arc::new(move |details| {
                        let mut state = connection_state.lock().unwrap();
                        state.connects.push(details.timeout);
                        assert_eq!(details.now, start + Duration::from_millis(state.elapsed));
                        state.elapsed += 20;
                        Ok(TimedTransport {
                            state: connection_state.clone(),
                            buffers: LazyBuffers::new(1024, 1024),
                            replies: VecDeque::from([
                                b"HTTP/1.1 200".as_slice(),
                                b" Connection",
                                b" established\r\n\r\n",
                            ]),
                        }
                        .boxed())
                    }),
                    register_shutdown: Arc::new(|_| Ok(())),
                };
                let result = ConnectProxyConnector::default().connect(&details, None::<()>);
                let state = state.lock().unwrap();
                assert_eq!(state.dns[0].after, Duration::from_millis(budget - 10));
                if dns_time == 100 {
                    assert!(state.connects.is_empty());
                    assert!(state.writes.is_empty());
                } else {
                    assert_eq!(state.connects[0].after, Duration::from_millis(budget - 30));
                    assert!(!state.writes.is_empty());
                    for timeout in &state.writes {
                        assert_eq!(timeout.after, Duration::from_millis(budget - 50));
                        assert_eq!(timeout.per_write, Some(Duration::from_secs(3)));
                    }
                    assert_eq!(
                        state
                            .reads
                            .iter()
                            .map(|t| t.after.as_millis())
                            .collect::<Vec<_>>(),
                        vec![
                            (budget - 50) as u128,
                            (budget - 70) as u128,
                            (budget - 90) as u128
                        ]
                    );
                    assert_eq!(state.elapsed, budget.min(110));
                }
                if budget == 200 {
                    assert!(result.is_ok());
                } else {
                    assert!(matches!(result, Err(Error::Timeout(actual)) if actual == reason));
                }
            }
        }
    }
}
