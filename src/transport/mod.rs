//! HTTP/1.1 data transport.
//!
//! _NOTE: Transport is deep configuration of ureq and is not required for regular use._
//!
//! ureq provides a pluggable transport layer making it possible to write bespoke
//! transports using the HTTP/1.1 protocol from point A to B. The
//! [`Agent::with_parts()`](crate::Agent::with_parts) constructor takes an implementation
//! of the [`Connector`] trait which is used for all connections made using that
//! agent.
//!
//! The [DefaultConnector] covers the regular needs for HTTP/1.1:
//!
//! * TCP Sockets
//! * SOCKS-proxy sockets
//! * HTTPS/TLS using rustls (feature flag **rustls**)
//! * HTTPS/TLS using native-tls (feature flag **native-tls** + [config](crate::tls::TlsProvider::NativeTls))
//!
//! The [`Connector`] trait anticipates a chain of connectors that each decide
//! whether to help perform the connection or not. It is for instance possible to make a
//! connector handling other schemes than `http`/`https` without affecting "regular" connections
//! using these schemes. See [`ChainedConnector`] for a helper connector that aids setting
//! up a chain of concrete connectors.

use std::fmt::Debug;

use http::uri::Scheme;
use http::Uri;

use crate::config::Config;
use crate::http;
use crate::proxy::Proto;
use crate::resolver::{ResolvedSocketAddrs, Resolver};
use crate::Error;

pub use self::tcp::TcpConnector;
use self::time::Instant;

mod buf;
pub use buf::{Buffers, LazyBuffers};

mod tcp;

mod io;
pub use io::TransportAdapter;

mod chain;
pub use chain::ChainedConnector;

#[cfg(feature = "_test")]
mod test;
#[cfg(feature = "_test")]
pub use test::set_handler;

#[cfg(feature = "socks-proxy")]
mod socks;
#[cfg(feature = "socks-proxy")]
pub use self::socks::SocksConnector;

pub use crate::proxy::ConnectProxyConnector;

pub mod time;

pub use crate::timings::NextTimeout;

/// Trait for components providing some aspect of connecting.
///
/// A connector instance is reused to produce multiple [`Transport`] instances (where `Transport`
/// instance would typically be a socket connection).
///
/// A connector can be part of a chain of connectors. The [`DefaultConnector`] provides a chain that
/// first tries to make a concrete socket connection (using [`TcpConnector`]) and then pass the
/// resulting [`Transport`] to a TLS wrapping connector
/// (see [`RustlsConnector`](crate::tls::RustlsConnector)). This makes it possible combine connectors
/// in new ways. A user of ureq could implement bespoke connector (such as SCTP) and still use
/// the `RustlsConnector` to wrap the underlying transport in TLS.
///
/// The built-in connectors provide SOCKS, TCP sockets and TLS wrapping.
pub trait Connector: Debug + Send + Sync + 'static {
    /// Helper to quickly box a transport.
    #[doc(hidden)]
    fn boxed(self) -> Box<dyn Connector>
    where
        Self: Sized,
    {
        Box::new(self)
    }

    /// Try to use this connector
    ///
    /// * The [`ConnectionDetails`] parameter encapsulates config and the specific details of
    ///   the connection being made currently (such as the [`Uri`]).
    /// * The `chained` parameter is used for connector chains and contains the [`Transport`]
    ///   instantiated one of the previous connectors in the chain. All `Connector` instances
    ///   can decide whether they want to pass this `Transport` along as is, wrap it in something
    ///   like TLS or even ignore it to provide some other connection instead.
    ///
    /// Return the `Transport` as produced by this connector, which could be just
    /// the incoming `chained` argument.
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error>;
}

/// The parameters needed to create a [`Transport`].
pub struct ConnectionDetails<'a> {
    /// Full uri that is being requested.
    pub uri: &'a Uri,

    /// The resolved IP address + port for the uri being requested. See [`Resolver`].
    ///
    /// For CONNECT proxy, this is the address of the proxy server.
    pub addrs: ResolvedSocketAddrs,

    /// The Agent configuration.
    pub config: &'a Config,

    /// The resolver configured on [`Agent`](crate::Agent).
    ///
    /// Typically the IP address of the host in the uri is already resolved to the `addr`
    /// property. However there might be cases where additional DNS lookups need to be
    /// made in the connector itself, such as resolving a SOCKS proxy server.
    pub resolver: &'a dyn Resolver,

    /// Current time.
    pub now: Instant,

    /// The next timeout for making the connection.
    // TODO(martin): Make mechanism to lower duration for each step in the connector chain.
    pub timeout: NextTimeout,
}

impl<'a> ConnectionDetails<'a> {
    /// Tell if the requested socket need TLS wrapping.
    ///
    /// This is (obviously) true for URLs starting `https`, but
    /// also in the case of using a CONNECT proxy over https.
    pub fn needs_tls(&self) -> bool {
        if let Some(p) = &self.config.proxy {
            if p.proto() == Proto::Https {
                return true;
            }
        }

        self.uri.scheme() == Some(&Scheme::HTTPS)
    }
}

/// Transport of HTTP/1.1 as created by a [`Connector`].
///
/// In ureq, [`Transport`] and [`Buffers`] go hand in hand. The rest of ureq tries to minimize
/// the allocations, and the transport is responsible for providing the buffers required
/// to perform the request. Unless the transport requires special buffer handling, the
/// [`LazyBuffers`] implementation can be used.
///
/// For sending data, the order of calls are:
///
/// 1. [`Transport::buffers()`] to obtain the buffers.
/// 2. [`Buffers::output()`] or [`Buffers::tmp_and_output`]
///    depending where in the life cycle of the request ureq is.
/// 3. [`Transport::transmit_output()`] to ask the transport to send/flush the `amount` of
///    buffers used in 2.
///
/// For receiving data, the order of calls are:
///
/// 1. [`Transport::await_input()`]
/// 2. The transport impl itself uses [`Buffers::input_append_buf()`] to fill a number
///    of bytes from the underlying transport and use [`Buffers::input_appended()`] to
///    tell the buffer how much been filled.
/// 3. [`Transport::buffers()`] to obtain the buffers
/// 4. [`Buffers::input()`] followed by [`Buffers::input_consume()`]. It's important to retain the
///    unconsumed bytes for the next call to `await_input()`. This is handled by [`LazyBuffers`].
///    It's important to call [`Buffers::input_consume()`] also with 0 consumed bytes since that's
///    how we keep track of whether the input is making progress.
///
pub trait Transport: Debug + Send + Sync {
    /// Provide buffers for this transport.
    fn buffers(&mut self) -> &mut dyn Buffers;

    /// Transmit `amount` of the output buffer. ureq will always transmit the entirety
    /// of the data written to the output buffer. It is expected that the transport will
    /// transmit the entire requested `amount`.
    ///
    /// The timeout should be used to abort the transmission if the amount can't be written in time.
    /// If that happens the transport must return an [`Error::Timeout`] instance.
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error>;

    /// Await input from the transport. The transport should internally use
    /// [`Buffers::input_append_buf()`] followed by [`Buffers::input_appended()`] to
    /// store the incoming data.
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error>;

    /// Tell whether this transport is still functional. This must provide an accurate answer
    /// for connection pooling to work.
    fn is_open(&mut self) -> bool;

    /// Whether the transport is TLS.
    ///
    /// Defaults to `false`, override in TLS transports.
    fn is_tls(&self) -> bool {
        false
    }
}

/// Default connector providing TCP sockets, TLS and SOCKS proxy.
///
/// This connector is a [`ChainedConnector`] with the following chain:
///
/// 1. [`SocksConnector`] to handle proxy settings if set.
/// 2. [`TcpConnector`] to open a socket directly if a proxy is not used.
/// 3. [`RustlsConnector`](crate::tls::RustlsConnector) which wraps the
///    connection from 1 or 2 in TLS if the scheme is `https` and the
///    [`TlsConfig`](crate::tls::TlsConfig) indicate we are using **rustls**.
///    This is the default TLS provider.
/// 4. [`NativeTlsConnector`](crate::tls::NativeTlsConnector) which wraps
///    the connection from 1 or 2 in TLS if the scheme is `https` and
///    [`TlsConfig`](crate::tls::TlsConfig) indicate we are using **native-tls**.
///
#[derive(Debug)]
pub struct DefaultConnector {
    chain: ChainedConnector,
}

impl DefaultConnector {
    /// Creates a default connector.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DefaultConnector {
    fn default() -> Self {
        let chain = ChainedConnector::new([
            //
            // When enabled, all tests are connected to a dummy server and will not
            // make requests to the internet.
            #[cfg(feature = "_test")]
            test::TestConnector.boxed(),
            //
            // If we are using socks-proxy, that takes precedence over TcpConnector.
            #[cfg(feature = "socks-proxy")]
            SocksConnector::default().boxed(),
            //
            // If the config indicates we ought to use a socks proxy
            // and the feature flag isn't enabled, we should warn the user.
            #[cfg(not(feature = "socks-proxy"))]
            no_proxy::WarnOnNoSocksConnector.boxed(),
            //
            // If we didn't get a socks-proxy, open a Tcp connection
            TcpConnector::default().boxed(),
            //
            // If rustls is enabled, prefer that
            #[cfg(feature = "rustls")]
            crate::tls::RustlsConnector::default().boxed(),
            //
            // Panic if the config calls for rustls, the uri scheme is https and that
            // TLS provider is not enabled by feature flags.
            #[cfg(feature = "_tls")]
            no_tls::WarnOnMissingTlsProvider(crate::tls::TlsProvider::Rustls).boxed(),
            //
            // As a fallback if rustls isn't enabled, use native-tls
            #[cfg(feature = "native-tls")]
            crate::tls::NativeTlsConnector::default().boxed(),
            //
            // Panic if the config calls for native-tls, the uri scheme is https and that
            // TLS provider is not enabled by feature flags.
            #[cfg(feature = "_tls")]
            no_tls::WarnOnMissingTlsProvider(crate::tls::TlsProvider::NativeTls).boxed(),
            //
            // Do the final CONNECT proxy on top of the connection if indicated by config.
            ConnectProxyConnector.boxed(),
        ]);

        DefaultConnector { chain }
    }
}

impl Connector for DefaultConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        self.chain.connect(details, chained)
    }
}

#[cfg(not(feature = "socks-proxy"))]
mod no_proxy {
    use super::{ConnectionDetails, Connector, Debug, Error, Transport};

    #[derive(Debug)]
    pub(crate) struct WarnOnNoSocksConnector;

    impl Connector for WarnOnNoSocksConnector {
        fn connect(
            &self,
            details: &ConnectionDetails,
            chained: Option<Box<dyn Transport>>,
        ) -> Result<Option<Box<dyn Transport>>, Error> {
            if chained.is_none() {
                if let Some(proxy) = &details.config.proxy {
                    if proxy.proto().is_socks() {
                        if proxy.is_from_env() {
                            warn!(
                                "Enable feature socks-proxy to use proxy
                                configured by environment variables"
                            );
                        } else {
                            // If a user bothered to manually create a Config.proxy setting,
                            // and it's not honored, assume it's a serious error.
                            panic!(
                                "Enable feature socks-proxy to use
                                manually configured proxy"
                            );
                        }
                    }
                }
            }
            Ok(chained)
        }
    }
}

#[cfg(feature = "_tls")]
mod no_tls {
    use crate::tls::TlsProvider;

    use super::{ConnectionDetails, Connector, Debug, Error, Transport};

    #[derive(Debug)]
    pub(crate) struct WarnOnMissingTlsProvider(pub TlsProvider);

    impl Connector for WarnOnMissingTlsProvider {
        fn connect(
            &self,
            details: &ConnectionDetails,
            chained: Option<Box<dyn Transport>>,
        ) -> Result<Option<Box<dyn Transport>>, Error> {
            let already_tls = chained.as_ref().map(|c| c.is_tls()).unwrap_or(false);

            if already_tls {
                return Ok(chained);
            }

            let tls_config = &details.config.tls_config;

            if details.needs_tls() && tls_config.provider == self.0 && !self.0.is_feature_enabled()
            {
                panic!(
                    "uri scheme is https, provider is {:?} but feature is not enabled: {}",
                    self.0,
                    self.0.feature_name()
                );
            }

            Ok(chained)
        }
    }
}
