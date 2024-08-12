//! TLS for handling `https`.

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
/// Defaults to [`RustlsWithRing`][Self::RustlsWithRing] because this has the highest chance
/// to compile and "just work" straight out of the box without installing additional
/// development dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TlsProvider {
    /// [Rustls](https://crates.io/crates/rustls) with [Ring](https://crates.io/crates/ring) as
    /// cryptographic backend.
    ///
    /// Requires the feature flag **rustls**.
    ///
    /// This is the default.
    RustlsWithRing,

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
            TlsProvider::RustlsWithRing => {
                cfg!(feature = "rustls")
            }
            TlsProvider::NativeTls => {
                cfg!(feature = "native-tls")
            }
        }
    }

    pub(crate) fn feature_name(&self) -> &'static str {
        match self {
            TlsProvider::RustlsWithRing => "rustls",
            TlsProvider::NativeTls => "native-tls",
        }
    }
}

/// Configuration of TLS.
///
/// This configuration is in common for both the different TLS mechanisms (available through
/// feature flags **rustls** and **native-tls**).
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// The provider to use.
    ///
    /// Defaults to [`TlsProvider::RustlsWithRing`].
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
        }
    }
}

impl Default for TlsProvider {
    fn default() -> Self {
        Self::RustlsWithRing
    }
}
