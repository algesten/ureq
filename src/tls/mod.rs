//! TLS for handling `https`.

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod cert;
pub use cert::{Certificate, PemItem, PrivateKey, parse_pem};

#[cfg(feature = "_rustls")]
pub(crate) mod rustls;

#[cfg(feature = "native-tls")]
pub(crate) mod native_tls;

/// Setting for which TLS provider to use.
///
/// Defaults to [`Rustls`][Self::Rustls] because this has the highest chance
/// to compile and "just work" straight out of the box without installing additional
/// development dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TlsProvider {
    /// [Rustls](https://crates.io/crates/rustls) with the
    /// [process-wide default cryptographic backend](https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html#method.install_default),
    /// or [Ring](https://crates.io/crates/ring) if no process-wide default is set.
    ///
    /// Requires the feature flag **rustls**.
    ///
    /// This is the default.
    #[default]
    Rustls,

    /// [Native-TLS](https://crates.io/crates/native-tls) for cases where it's important to
    /// use the TLS libraries installed on the host running ureq.
    ///
    /// Requires the feature flag **native-tls** and that using an [`Agent`](crate::Agent) with
    /// this config option set in the [`TlsConfig`].
    ///
    /// The setting is never picked up automatically.
    NativeTls,
}

impl TlsProvider {
    pub(crate) fn is_feature_enabled(&self) -> bool {
        match self {
            TlsProvider::Rustls => {
                cfg!(feature = "_rustls")
            }
            TlsProvider::NativeTls => {
                cfg!(feature = "native-tls")
            }
        }
    }

    pub(crate) fn feature_name(&self) -> &'static str {
        match self {
            TlsProvider::Rustls => "rustls",
            TlsProvider::NativeTls => "native-tls",
        }
    }
}

/// Configuration of TLS.
///
/// This configuration is in common for both the different TLS mechanisms (available through
/// feature flags **rustls** and **native-tls**).
#[derive(Clone)]
pub struct TlsConfig {
    provider: TlsProvider,
    client_cert: Option<ClientCert>,
    root_certs: RootCerts,
    use_sni: bool,
    disable_verification: bool,
    #[cfg(feature = "_rustls")]
    rustls_crypto_provider: Option<Arc<::rustls::crypto::CryptoProvider>>,
}

impl TlsConfig {
    /// Builder to make a bespoke config.
    pub fn builder() -> TlsConfigBuilder {
        TlsConfigBuilder {
            config: TlsConfig::default(),
        }
    }

    pub(crate) fn hash_value(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

impl TlsConfig {
    /// The provider to use.
    ///
    /// Defaults to [`TlsProvider::Rustls`].
    pub fn provider(&self) -> TlsProvider {
        self.provider
    }

    /// Client certificate chain with corresponding private key.
    ///
    /// Defaults to `None`.
    pub fn client_cert(&self) -> Option<&ClientCert> {
        self.client_cert.as_ref()
    }

    /// The set of trusted root certificates to use to validate server certificates.
    ///
    /// Defaults to `WebPki`.
    pub fn root_certs(&self) -> &RootCerts {
        &self.root_certs
    }

    /// Whether to send SNI (Server Name Indication) to the remote server.
    ///
    /// This is used by the server to determine which domain/certificate we are connecting
    /// to for servers where multiple domains/sites are hosted on the same IP.
    ///
    /// Defaults to `true`.
    pub fn use_sni(&self) -> bool {
        self.use_sni
    }

    /// **WARNING** Disable all server certificate verification.
    ///
    /// This breaks encryption and leaks secrets. Must never be enabled for code where
    /// any level of security is required.
    pub fn disable_verification(&self) -> bool {
        self.disable_verification
    }

    /// Specific `CryptoProvider` to use for `rustls`.
    ///
    /// # UNSTABLE API
    ///
    /// **NOTE: This API is not guaranteed for semver.**
    ///
    /// `rustls` is not (yet) semver 1.x and ureq can't promise that this API is upheld.
    /// If `rustls` makes a breaking change regarding `CryptoProvider` their configuration,
    /// or incompatible data types between rustls versions, ureq will _NOT_ bump a major version.
    ///
    /// ureq will update to the latest `rustls` minor version using ureq minor versions.
    #[cfg(feature = "_rustls")]
    pub fn unversioned_rustls_crypto_provider(
        &self,
    ) -> &Option<Arc<::rustls::crypto::CryptoProvider>> {
        &self.rustls_crypto_provider
    }
}

/// Builder of [`TlsConfig`]
pub struct TlsConfigBuilder {
    config: TlsConfig,
}

impl TlsConfigBuilder {
    /// The provider to use.
    ///
    /// Defaults to [`TlsProvider::Rustls`].
    pub fn provider(mut self, v: TlsProvider) -> Self {
        self.config.provider = v;
        self
    }

    /// Client certificate chain with corresponding private key.
    ///
    /// Defaults to `None`.
    pub fn client_cert(mut self, v: Option<ClientCert>) -> Self {
        self.config.client_cert = v;
        self
    }

    /// The set of trusted root certificates to use to validate server certificates.
    ///
    /// Defaults to `WebPki`.
    pub fn root_certs(mut self, v: RootCerts) -> Self {
        self.config.root_certs = v;
        self
    }

    /// Whether to send SNI (Server Name Indication) to the remote server.
    ///
    /// This is used by the server to determine which domain/certificate we are connecting
    /// to for servers where multiple domains/sites are hosted on the same IP.
    ///
    /// Defaults to `true`.
    pub fn use_sni(mut self, v: bool) -> Self {
        self.config.use_sni = v;
        self
    }

    /// **WARNING** Disable all server certificate verification.
    ///
    /// This breaks encryption and leaks secrets. Must never be enabled for code where
    /// any level of security is required.
    pub fn disable_verification(mut self, v: bool) -> Self {
        self.config.disable_verification = v;
        self
    }

    /// Specific `CryptoProvider` to use for `rustls`.
    ///
    /// # UNSTABLE API
    ///
    /// **NOTE: This API is not guaranteed for semver.**
    ///
    /// `rustls` is not (yet) semver 1.x and ureq can't promise that this API is upheld.
    /// If `rustls` makes a breaking change regarding `CryptoProvider` their configuration,
    /// or incompatible data types between rustls versions, ureq will _NOT_ bump a major version.
    ///
    /// ureq will update to the latest `rustls` minor version using ureq minor versions.
    ///
    /// # Feature flags
    ///
    /// This requires either feature **rustls** or **rustls-no-provider**, you probably
    /// want the latter when configuring an explicit crypto provider since
    /// **rustls** compiles with `ring`, while **rustls-no-provider** does not.
    ///
    /// # Example
    ///
    /// This example uses `aws-lc-rs` for the [`Agent`][crate::Agent]. The following
    /// depdendencies would compile ureq without `ring` and only aws-lc-rs.
    ///
    /// * `Cargo.toml`
    ///
    /// ```text
    /// ureq = { version = "3", default-features = false, features = ["rustls-no-provider"] }
    /// rustls = { version = "0.23", features = ["aws-lc-rs"] }
    /// ```
    ///
    /// * Agent
    ///
    /// ```
    /// use std::sync::Arc;
    /// use ureq::{Agent};
    /// use ureq::tls::{TlsConfig, TlsProvider};
    /// use rustls::crypto;
    ///
    /// let crypto = Arc::new(crypto::aws_lc_rs::default_provider());
    ///
    /// let agent = Agent::config_builder()
    ///     .tls_config(
    ///         TlsConfig::builder()
    ///             .provider(TlsProvider::Rustls)
    ///             // requires rustls or rustls-no-provider feature
    ///             .unversioned_rustls_crypto_provider(crypto)
    ///             .build()
    ///    )
    ///    .build()
    ///    .new_agent();
    /// ```
    #[cfg(feature = "_rustls")]
    pub fn unversioned_rustls_crypto_provider(
        mut self,
        v: Arc<::rustls::crypto::CryptoProvider>,
    ) -> Self {
        self.config.rustls_crypto_provider = Some(v);
        self
    }

    /// Finalize the config
    pub fn build(self) -> TlsConfig {
        self.config
    }
}

/// A client certificate.
#[derive(Debug, Clone, Hash)]
pub struct ClientCert(Arc<(Vec<Certificate<'static>>, PrivateKey<'static>)>);

impl ClientCert {
    /// Creates a new client certificate from a chain and a private key.
    pub fn new_with_certs(chain: &[Certificate<'static>], key: PrivateKey<'static>) -> Self {
        Self(Arc::new((chain.to_vec(), key)))
    }

    /// Client certificate chain.
    pub fn certs(&self) -> &[Certificate<'static>] {
        &self.0.0
    }

    /// Client certificate private key.
    pub fn private_key(&self) -> &PrivateKey<'static> {
        &self.0.1
    }
}

/// Configuration setting for root certs.
#[derive(Debug, Clone, Hash)]
#[non_exhaustive]
pub enum RootCerts {
    /// Use these specific certificates as root certs.
    Specific(Arc<Vec<Certificate<'static>>>),

    /// Use the platform's verifier.
    ///
    /// * For **rustls**, this uses the `rustls-platform-verifier` crate. It requires
    ///   the feature **platform-verifier**.
    /// * For **native-tls**, this uses the roots that native-tls loads by default.
    PlatformVerifier,

    /// Use Mozilla's root certificates instead of the platform.
    ///
    /// This is useful when you can't trust the system roots, such as in
    /// environments where TLS is intercepted and decrypted by a proxy (MITM attack).
    ///
    /// This is the default value.
    WebPki,
}

impl RootCerts {
    /// Use these specific root certificates
    pub fn new_with_certs(certs: &[Certificate<'static>]) -> Self {
        certs.iter().cloned().into()
    }
}

impl<I: IntoIterator<Item = Certificate<'static>>> From<I> for RootCerts {
    fn from(value: I) -> Self {
        RootCerts::Specific(Arc::new(value.into_iter().collect()))
    }
}

impl Default for TlsConfig {
    fn default() -> Self {
        let provider = TlsProvider::default();
        Self {
            provider,
            client_cert: None,
            root_certs: RootCerts::WebPki,
            use_sni: true,
            disable_verification: false,
            #[cfg(feature = "_rustls")]
            rustls_crypto_provider: None,
        }
    }
}

impl fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConfig")
            .field("provider", &self.provider)
            .field("client_cert", &self.client_cert)
            .field("root_certs", &self.root_certs)
            .field("use_sni", &self.use_sni)
            .field("disable_verification", &self.disable_verification)
            .finish()
    }
}

impl Hash for TlsConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.provider.hash(state);
        self.client_cert.hash(state);
        self.root_certs.hash(state);
        self.use_sni.hash(state);
        self.disable_verification.hash(state);

        #[cfg(feature = "_rustls")]
        if let Some(arc) = &self.rustls_crypto_provider {
            (Arc::as_ptr(arc) as usize).hash(state);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use assert_no_alloc::*;

    #[test]
    fn tls_config_clone_does_not_allocate() {
        let c = TlsConfig::default();
        assert_no_alloc(|| c.clone());
    }

    #[cfg(any(feature = "_rustls", feature = "native-tls"))]
    mod handshake {
        use std::sync::Arc;

        use crate::tls::{TlsConfig, TlsProvider};
        use crate::transport::time::{Duration, Instant};
        use crate::transport::{Buffers, ConnectionDetails, Connector, LazyBuffers};
        use crate::transport::{NextTimeout, Transport};
        use crate::unversioned::resolver::{DefaultResolver, Resolver};
        use crate::{Agent, Error, Timeout};

        #[derive(Debug)]
        struct TimeoutTransport {
            buffers: LazyBuffers,
            timeout: NextTimeout,
        }

        impl Transport for TimeoutTransport {
            fn buffers(&mut self) -> &mut dyn Buffers {
                &mut self.buffers
            }

            fn transmit_output(
                &mut self,
                _amount: usize,
                timeout: NextTimeout,
            ) -> Result<(), Error> {
                assert_eq!(timeout, self.timeout);
                Err(Error::Timeout(timeout.reason))
            }

            fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
                assert_eq!(timeout, self.timeout);
                Err(Error::Timeout(timeout.reason))
            }

            fn is_open(&mut self) -> bool {
                true
            }
        }

        fn check_timeout(connector: impl Connector<TimeoutTransport>, tls_config: TlsConfig) {
            let config = Agent::config_builder()
                .proxy(None)
                .tls_config(tls_config)
                .build();
            let uri = "https://example.com/".parse().unwrap();
            let resolver = DefaultResolver::default();

            for reason in [Timeout::Connect, Timeout::Global] {
                let timeout = NextTimeout {
                    after: Duration::from_secs(7),
                    reason,
                };
                let details = ConnectionDetails {
                    uri: &uri,
                    addrs: resolver.empty(),
                    resolver: &resolver,
                    config: &config,
                    request_level: false,
                    now: Instant::now(),
                    timeout,
                    current_time: Arc::new(Instant::now),
                    run_connector: Arc::new(|_| unreachable!("TLS must use the chained transport")),
                };
                let transport = TimeoutTransport {
                    buffers: LazyBuffers::new(1024, 1024),
                    timeout,
                };

                let error = connector.connect(&details, Some(transport)).unwrap_err();
                assert!(
                    matches!(error, Error::Timeout(actual) if actual == reason),
                    "{error:?}"
                );
            }
        }

        #[test]
        #[cfg(feature = "_rustls")]
        fn rustls_uses_connection_timeout() {
            let tls_config = TlsConfig::builder()
                .provider(TlsProvider::Rustls)
                .disable_verification(true)
                .unversioned_rustls_crypto_provider(Arc::new(
                    ::rustls::crypto::aws_lc_rs::default_provider(),
                ))
                .build();
            check_timeout(crate::tls::rustls::RustlsConnector::default(), tls_config);
        }

        #[test]
        #[cfg(feature = "native-tls")]
        fn native_tls_uses_connection_timeout() {
            let tls_config = TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .disable_verification(true)
                .build();
            check_timeout(
                crate::tls::native_tls::NativeTlsConnector::default(),
                tls_config,
            );
        }
    }
}
