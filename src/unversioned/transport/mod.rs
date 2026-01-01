//! HTTP/1.1 data transport.
//!
//! **NOTE: transport does not (yet) [follow semver][super].**
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
//! connector handling other schemes than `http`/`https` without affecting "regular" connections.

use std::fmt::Debug;
use std::sync::Arc;

use http::uri::Scheme;
use http::Uri;

use crate::config::Config;
use crate::http;
use crate::Error;

use super::resolver::{ResolvedSocketAddrs, Resolver};

mod buf;
pub use buf::{Buffers, LazyBuffers};

mod tcp;
pub use self::tcp::TcpConnector;

mod io;
pub use io::TransportAdapter;

mod chain;
pub use chain::{ChainedConnector, Either};

mod connect;
pub use connect::ConnectProxyConnector;

#[cfg(feature = "_test")]
mod test;
#[cfg(feature = "_test")]
pub use test::{set_handler, set_handler_cb};

#[cfg(feature = "socks-proxy")]
mod socks;
#[cfg(feature = "socks-proxy")]
pub use self::socks::SocksConnector;

#[cfg(feature = "_rustls")]
pub use crate::tls::rustls::RustlsConnector;

#[cfg(feature = "native-tls")]
pub use crate::tls::native_tls::NativeTlsConnector;

pub mod time;
use self::time::Instant;

pub use crate::timings::NextTimeout;

/// Trait for components providing some aspect of connecting.
///
/// A connector instance is reused to produce multiple [`Transport`] instances (where `Transport`
/// instance would typically be a socket connection).
///
/// A connector can be part of a chain of connectors. The [`DefaultConnector`] provides a chain that
/// first tries to make a concrete socket connection (using [`TcpConnector`]) and then pass the
/// resulting [`Transport`] to a TLS wrapping connector
/// (see [`RustlsConnector`]). This makes it possible combine connectors
/// in new ways. A user of ureq could implement bespoke connector (such as SCTP) and still use
/// the `RustlsConnector` to wrap the underlying transport in TLS.
///
/// The built-in [`DefaultConnector`] provides SOCKS, TCP sockets and TLS wrapping.
///
/// # Errors
///
/// When writing a bespoke connector chain we recommend handling errors like this:
///
/// 1. Map to [`Error::Io`] as far as possible.
/// 2. Map to any other [`Error`] where reasonable.
/// 3. Fall back on [`Error::Other`] preserving the original error.
/// 4. As a last resort [`Error::ConnectionFailed`] + logging.
///
/// # Example
///
/// ```
/// # #[cfg(all(feature = "rustls", not(feature = "_test")))] {
/// use ureq::{Agent, config::Config};
///
/// // These types are not covered by the promises of semver (yet)
/// use ureq::unversioned::transport::{Connector, TcpConnector, RustlsConnector};
/// use ureq::unversioned::resolver::DefaultResolver;
///
/// // A connector chain that opens a TCP transport, then wraps it in a TLS.
/// let connector = ()
///     .chain(TcpConnector::default())
///     .chain(RustlsConnector::default());
///
/// let config = Config::default();
/// let resolver = DefaultResolver::default();
///
/// // Creates an agent with a bespoke connector
/// let agent = Agent::with_parts(config, connector, resolver);
///
/// let mut res = agent.get("https://httpbin.org/get").call().unwrap();
/// let body = res.body_mut().read_to_string().unwrap();
/// # }
/// ```
pub trait Connector<In: Transport = ()>: Debug + Send + Sync + 'static {
    /// The type of transport produced by this connector.
    type Out: Transport;

    /// Use this connector to make a [`Transport`].
    ///
    /// * The [`ConnectionDetails`] parameter encapsulates config and the specific details of
    ///   the connection being made currently (such as the [`Uri`]).
    /// * The `chained` parameter is used for connector chains and contains the [`Transport`]
    ///   instantiated one of the previous connectors in the chain. All `Connector` instances
    ///   can decide whether they want to pass this `Transport` along as is, wrap it in something
    ///   like TLS or even ignore it to provide some other connection instead.
    ///
    /// Returns the [`Transport`] as produced by this connector, which could be just
    /// the incoming `chained` argument.
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error>;

    /// Chain this connector to another connector.
    ///
    /// This connector will be called first, and the output goes into the next connector.
    fn chain<Next: Connector<Self::Out>>(self, next: Next) -> ChainedConnector<In, Self, Next>
    where
        Self: Sized,
    {
        ChainedConnector::new(self, next)
    }
}

/// Box a connector to erase the types.
///
/// This is typically used after the chain of connectors is set up.
pub(crate) fn boxed_connector<In, C>(c: C) -> Box<dyn Connector<In, Out = Box<dyn Transport>>>
where
    In: Transport,
    C: Connector<In>,
{
    #[derive(Debug)]
    struct BoxingConnector;

    impl<In: Transport> Connector<In> for BoxingConnector {
        type Out = Box<dyn Transport>;

        fn connect(
            &self,
            _: &ConnectionDetails,
            chained: Option<In>,
        ) -> Result<Option<Self::Out>, Error> {
            if let Some(transport) = chained {
                Ok(Some(Box::new(transport)))
            } else {
                Ok(None)
            }
        }
    }

    Box::new(c.chain(BoxingConnector))
}

/// The parameters needed to create a [`Transport`].
pub struct ConnectionDetails<'a> {
    /// Full uri that is being requested.
    ///
    /// In the case of CONNECT (HTTP) proxy, this is the URI of the
    /// proxy, and the actual URI is in the `proxied` field.
    pub uri: &'a Uri,

    /// The resolved IP address + port for the uri being requested. See [`Resolver`].
    ///
    /// For proxies, whetherh this holds real addresses depends on
    /// [`Proxy::resolve_target()`](crate::Proxy::resolve_target).
    pub addrs: ResolvedSocketAddrs,

    /// The configuration.
    ///
    /// Agent or Request level.
    pub config: &'a Config,

    /// Whether the config is request level.
    pub request_level: bool,

    /// The resolver configured on [`Agent`](crate::Agent).
    ///
    /// Typically the IP address of the host in the uri is already resolved to the `addr`
    /// property. However there might be cases where additional DNS lookups need to be
    /// made in the connector itself, such as resolving a SOCKS proxy server.
    pub resolver: &'a dyn Resolver,

    /// Current time.
    ///
    /// Time the ConnectionDetails was created.
    pub now: Instant,

    /// The next timeout for making the connection.
    // TODO(martin): Make mechanism to lower duration for each step in the connector chain.
    pub timeout: NextTimeout,

    /// Provides the current time.
    pub current_time: Arc<dyn Fn() -> Instant + Send + Sync + 'static>,

    /// Run the connector chain.
    ///
    /// Used for CONNECT proxy to establish a connection to the proxy server itself.
    pub run_connector: Arc<RunConnector>,
}

pub(crate) type RunConnector =
    dyn Fn(&ConnectionDetails) -> Result<Box<dyn Transport>, Error> + Send + Sync;

impl<'a> ConnectionDetails<'a> {
    /// Tell if the requested socket need TLS wrapping.
    pub fn needs_tls(&self) -> bool {
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
/// 1. [`Transport::maybe_await_input()`]
/// 2. The transport impl itself uses [`Buffers::input_append_buf()`] to fill a number
///    of bytes from the underlying transport and use [`Buffers::input_appended()`] to
///    tell the buffer how much been filled.
/// 3. [`Transport::buffers()`] to obtain the buffers
/// 4. [`Buffers::input()`] followed by [`Buffers::input_consume()`]. It's important to retain the
///    unconsumed bytes for the next call to `maybe_await_input()`. This is handled by [`LazyBuffers`].
///    It's important to call [`Buffers::input_consume()`] also with 0 consumed bytes since that's
///    how we keep track of whether the input is making progress.
///
pub trait Transport: Debug + Send + Sync + 'static {
    /// Provide buffers for this transport.
    fn buffers(&mut self) -> &mut dyn Buffers;

    /// Transmit `amount` of the output buffer. ureq will always transmit the entirety
    /// of the data written to the output buffer. It is expected that the transport will
    /// transmit the entire requested `amount`.
    ///
    /// The timeout should be used to abort the transmission if the amount can't be written in time.
    /// If that happens the transport must return an [`Error::Timeout`] instance.
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error>;

    /// Await input from the transport.
    ///
    /// Early returns if [`Buffers::can_use_input()`], return true.
    #[doc(hidden)]
    fn maybe_await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        // If we already have input available, we don't wait.
        // This might be false even when there is input in the buffer
        // because the last use of the buffer made no progress.
        // Example: we might want to read the _entire_ http request headers,
        //          not partially.
        if self.buffers().can_use_input() {
            return Ok(true);
        }

        self.await_input(timeout)
    }

    /// Wait for input and fill the buffer.
    ///
    /// 1. Use [`Buffers::input_append_buf()`] to fill the buffer
    /// 2. Followed by [`Buffers::input_appended()`] to report how many bytes were read.
    ///
    /// Returns `true` if it made progress, i.e. if it managed to fill the input buffer with any bytes.
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

    /// Turn this transport in a boxed version.
    // TODO(martin): is is complicating the public API?
    #[doc(hidden)]
    fn boxed(self) -> Box<dyn Transport>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }
}

/// Default connector providing TCP sockets, TLS and SOCKS proxy.
///
/// This connector is the following chain:
///
/// 1. [`SocksConnector`] to handle proxy settings if set.
/// 2. [`TcpConnector`] to open a socket directly if a proxy is not used.
/// 3. [`RustlsConnector`] which wraps the
///    connection from 1 or 2 in TLS if the scheme is `https` and the
///    [`TlsConfig`](crate::tls::TlsConfig) indicate we are using **rustls**.
///    This is the default TLS provider.
/// 4. [`NativeTlsConnector`] which wraps
///    the connection from 1 or 2 in TLS if the scheme is `https` and
///    [`TlsConfig`](crate::tls::TlsConfig) indicate we are using **native-tls**.
///
#[derive(Debug)]
pub struct DefaultConnector {
    inner: Box<dyn Connector<(), Out = Box<dyn Transport>>>,
}

impl DefaultConnector {
    /// Creates a default connector.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DefaultConnector {
    fn default() -> Self {
        let inner = ();

        // When enabled, all tests are connected to a dummy server and will not
        // make requests to the internet.
        #[cfg(feature = "_test")]
        let inner = inner.chain(test::TestConnector);

        // If we are using socks-proxy, that takes precedence over TcpConnector.
        #[cfg(feature = "socks-proxy")]
        let inner = inner.chain(SocksConnector::default());

        // If the config indicates we ought to use a socks proxy
        // and the feature flag isn't enabled, we should warn the user.
        #[cfg(not(feature = "socks-proxy"))]
        let inner = inner.chain(no_proxy::WarnOnNoSocksConnector);

        // If this is a CONNECT proxy, we must "prepare" the socket
        // by setting up another connection and sending the `CONNECT host:port` line.
        let inner = inner.chain(ConnectProxyConnector::default());

        // If we didn't get a socks-proxy, open a Tcp connection
        let inner = inner.chain(TcpConnector::default());

        // If rustls is enabled, prefer that
        #[cfg(feature = "_rustls")]
        let inner = inner.chain(RustlsConnector::default());

        // Panic if the config calls for rustls, the uri scheme is https and that
        // TLS provider is not enabled by feature flags.
        #[cfg(feature = "_tls")]
        let inner = inner.chain(no_tls::WarnOnMissingTlsProvider(
            crate::tls::TlsProvider::Rustls,
        ));

        // As a fallback if rustls isn't enabled, use native-tls
        #[cfg(feature = "native-tls")]
        let inner = inner.chain(NativeTlsConnector::default());

        // Panic if the config calls for native-tls, the uri scheme is https and that
        // TLS provider is not enabled by feature flags.
        #[cfg(feature = "_tls")]
        let inner = inner.chain(no_tls::WarnOnMissingTlsProvider(
            crate::tls::TlsProvider::NativeTls,
        ));

        DefaultConnector {
            inner: boxed_connector(inner),
        }
    }
}

impl Connector<()> for DefaultConnector {
    type Out = Box<dyn Transport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, Error> {
        self.inner.connect(details, chained)
    }
}

#[cfg(not(feature = "socks-proxy"))]
mod no_proxy {
    use super::{ConnectionDetails, Connector, Debug, Error, Transport};

    #[derive(Debug)]
    pub(crate) struct WarnOnNoSocksConnector;

    impl<In: Transport> Connector<In> for WarnOnNoSocksConnector {
        type Out = In;

        fn connect(
            &self,
            details: &ConnectionDetails,
            chained: Option<In>,
        ) -> Result<Option<Self::Out>, Error> {
            if chained.is_none() {
                if let Some(proxy) = details.config.proxy() {
                    if proxy.protocol().is_socks() {
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

    impl<In: Transport> Connector<In> for WarnOnMissingTlsProvider {
        type Out = In;

        fn connect(
            &self,
            details: &ConnectionDetails,
            chained: Option<In>,
        ) -> Result<Option<Self::Out>, Error> {
            let already_tls = chained.as_ref().map(|c| c.is_tls()).unwrap_or(false);

            if already_tls {
                return Ok(chained);
            }

            let tls_config = details.config.tls_config();

            if details.needs_tls()
                && tls_config.provider() == self.0
                && !self.0.is_feature_enabled()
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

impl<T: Transport> Transport for Box<T>
where
    T: ?Sized,
{
    fn buffers(&mut self) -> &mut dyn Buffers {
        (**self).buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        (**self).transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        (**self).await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        (**self).is_open()
    }

    fn is_tls(&self) -> bool {
        (**self).is_tls()
    }
}
