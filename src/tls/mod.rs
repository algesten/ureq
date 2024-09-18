//! TLS for handling `https`.

use std::fmt;
use std::sync::Arc;

mod cert;
pub use cert::{parse_pem, Certificate, PemItem, PrivateKey};

#[cfg(feature = "rustls")]
mod rustls;
#[cfg(feature = "rustls")]
pub use self::rustls::RustlsConnector;

#[cfg(feature = "native-tls")]
mod native_tls;
#[cfg(feature = "native-tls")]
pub use self::native_tls::NativeTlsConnector;

/// Setting for which TLS provider to use.
///
/// Defaults to [`Rustls`][Self::Rustls] because this has the highest chance
/// to compile and "just work" straight out of the box without installing additional
/// development dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TlsProvider {
    /// [Rustls](https://crates.io/crates/rustls) with the
    /// [process-wide default cryptographic backend](https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html#method.install_default),
    /// or [Ring](https://crates.io/crates/ring) if no process-wide default is set.
    ///
    /// Requires the feature flag **rustls**.
    ///
    /// This is the default.
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
                cfg!(feature = "rustls")
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
    /// The provider to use.
    ///
    /// Defaults to [`TlsProvider::Rustls`].
    pub(crate) provider: TlsProvider,

    /// Client certificate chain with corresponding private key.
    ///
    /// Defaults to `None`.
    pub(crate) client_cert: Option<ClientCert>,

    /// The set of trusted root certificates to use to validate server certificates.
    ///
    /// Defaults to `WebPki`.
    pub(crate) root_certs: RootCerts,

    /// Whether to send SNI (Server Name Indication) to the remote server.
    ///
    /// This is used by the server to determine which domain/certificate we are connecting
    /// to for servers where multiple domains/sites are hosted on the same IP.
    ///
    /// Defaults to `true`.
    pub(crate) use_sni: bool,

    /// **WARNING** Disable all server certificate verification.
    ///
    /// This breaks encryption and leaks secrets. Must never be enabled for code where
    /// any level of security is required.
    pub(crate) disable_verification: bool,
}

impl TlsConfig {
    /// Builder to make a bespoke config.
    pub fn builder() -> TlsConfigBuilder {
        TlsConfigBuilder {
            config: TlsConfig::default(),
        }
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

    /// Finalize the config
    pub fn build(self) -> TlsConfig {
        self.config
    }
}

/// A client certificate.
#[derive(Debug, Clone)]
pub struct ClientCert(pub Arc<(Vec<Certificate<'static>>, PrivateKey<'static>)>);

impl ClientCert {
    /// Creates a new client certificate from a chain and a private key.
    pub fn new_with_certs(chain: &[Certificate<'static>], key: PrivateKey<'static>) -> Self {
        Self(Arc::new((chain.to_vec(), key)))
    }
}

/// Configuration setting for root certs.
#[derive(Debug, Clone)]
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
        }
    }
}

impl Default for TlsProvider {
    fn default() -> Self {
        Self::Rustls
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

#[cfg(test)]
mod test {
    use super::*;
    use assert_no_alloc::*;

    #[test]
    fn tls_config_clone_does_not_allocate() {
        let c = TlsConfig::default();
        assert_no_alloc(|| c.clone());
    }
}
