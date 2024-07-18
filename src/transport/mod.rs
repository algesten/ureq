use std::fmt::Debug;
use std::net::SocketAddr;

use http::Uri;

use crate::proxy::Proxy;
use crate::resolver::Resolver;
use crate::time::{Instant, NextTimeout};
use crate::tls::TlsProvider;
use crate::{AgentConfig, Error};

use self::tcp::TcpConnector;

mod buf;
pub(crate) use buf::NoBuffers;
pub use buf::{Buffers, LazyBuffers};

mod tcp;

mod io;
pub use io::TransportAdapter;

mod chain;
pub use chain::ChainedConnector;

#[cfg(feature = "_test")]
mod test;

#[cfg(feature = "socks-proxy")]
mod socks;
#[cfg(feature = "socks-proxy")]
pub use self::socks::SocksConnector;

pub trait Connector: Debug + Send + Sync + 'static {
    fn boxed(self) -> Box<dyn Connector>
    where
        Self: Sized,
    {
        Box::new(self)
    }

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error>;
}

pub struct ConnectionDetails<'a> {
    pub uri: &'a Uri,
    pub addr: SocketAddr,
    pub proxy: &'a Option<Proxy>,
    pub resolver: &'a dyn Resolver,
    pub config: &'a AgentConfig,

    pub now: Instant,
    // TODO(martin): Make mechanism to lower duration for each step in the connector chain.
    pub timeout: NextTimeout,
}

pub trait Transport: Debug + Send + Sync {
    fn buffers(&mut self) -> &mut dyn Buffers;
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error>;
    fn await_input(&mut self, timeout: NextTimeout) -> Result<(), Error>;
    fn consume_input(&mut self, amount: usize);
    fn is_open(&mut self) -> bool;
    fn is_tls(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct DefaultConnector {
    chain: ChainedConnector,
}

impl Default for DefaultConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultConnector {
    pub fn new() -> Self {
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
            TcpConnector.boxed(),
            //
            // If rustls is enabled, prefer that
            #[cfg(feature = "rustls")]
            crate::tls::RustlsConnector::default().boxed(),
            //
            // Panic if the config calls for rustls, the uri scheme is https and that
            // TLS provider is not enabled by feature flags.
            #[cfg(feature = "_tls")]
            no_tls::WarnOnMissingTlsProvider(TlsProvider::RustlsWithRing).boxed(),
            //
            // As a fallback if rustls isn't enabled, use native-tls
            #[cfg(feature = "native-tls")]
            crate::tls::NativeTlsConnector::default().boxed(),
            //
            // Panic if the config calls for native-tls, the uri scheme is https and that
            // TLS provider is not enabled by feature flags.
            #[cfg(feature = "_tls")]
            no_tls::WarnOnMissingTlsProvider(TlsProvider::NativeTls).boxed(),
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
                if let Some(proxy) = &details.proxy {
                    if proxy.proto().is_socks() {
                        if proxy.is_from_env() {
                            warn!(
                                "Enable feature socks-proxy to use proxy
                                configured by environment variables"
                            );
                        } else {
                            // If a user bothered to manually create a AgentConfig.proxy setting,
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
    use http::uri::Scheme;

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

            if details.uri.scheme() != Some(&Scheme::HTTPS)
                && tls_config.provider == self.0
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
