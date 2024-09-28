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
    pub provider: TlsProvider,

    /// Client certificate chains with corresponding private keys.
    ///
    /// Defaults to `None`.
    pub client_cert: Option<(Vec<Certificate<'static>>, Arc<PrivateKey<'static>>)>,

    /// The set of trusted root certificates to use to validate server certificates.
    ///
    /// Defaults to `PlatformVerifier` to use the platform default root certs.
    pub root_certs: RootCerts,

    /// Whether to send SNI (Server Name Indication) to the remote server.
    ///
    /// This is used by the server to determine which domain/certificate we are connecting
    /// to for servers where multiple domains/sites are hosted on the same IP.
    ///
    /// Defaults to `true`.
    pub use_sni: bool,

    /// **WARNING** Disable all server certificate verification.
    ///
    /// This breaks encryption and leaks secrets. Must never be enabled for code where
    /// any level of security is required.
    pub disable_verification: bool,

    // This is here to force users of ureq to use the ..Default::default() pattern
    // as part of creating `Config`. That way we can introduce new settings without
    // it becoming a breaking changes.
    #[doc(hidden)]
    pub _must_use_default: private::Private,
}

// Deliberately not publicly visible.
mod private {
    #[derive(Debug, Clone, Copy)]
    pub struct Private;
}

/// Configuration setting for root certs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RootCerts {
    /// Use these specific certificates as root certs.
    SpecificCerts(Vec<Certificate<'static>>),

    /// Use the platform's verifier.
    ///
    /// * For **rustls**, this uses the `rustls-platform-verifier` crate.
    /// * For **native-tls**, this uses the roots that native-tls loads by default.
    PlatformVerifier,

    /// Use Mozilla's root certificates instead of the platform.
    ///
    /// This is useful when you can't trust the system roots, such as in
    /// environments where TLS is intercepted and decrypted by a proxy (MITM attack).
    WebPki,
}

impl Default for TlsConfig {
    fn default() -> Self {
        let provider = TlsProvider::default();
        Self {
            provider,
            client_cert: None,
            root_certs: RootCerts::PlatformVerifier,
            use_sni: true,
            disable_verification: false,

            _must_use_default: private::Private,
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
