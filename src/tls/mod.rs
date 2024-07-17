mod cert;
use std::sync::Arc;

pub use cert::{parse_pem, Certificate, PemItem, PrivateKey};

use self::cert::load_root_certs;

#[cfg(feature = "rustls")]
mod rustls;
#[cfg(feature = "rustls")]
pub use self::rustls::RustlsConnector;

#[cfg(feature = "native-tls")]
mod native_tls;
#[cfg(feature = "native-tls")]
pub use self::native_tls::NativeTlsConnector;

#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub client_cert: Option<(Vec<Certificate<'static>>, Arc<PrivateKey<'static>>)>,
    pub root_certs: Vec<Certificate<'static>>,
    pub use_sni: bool,
    pub disable_verification: bool,
}

#[cfg(not(feature = "native-roots"))]
impl TlsConfig {
    pub fn with_native_roots() -> TlsConfig {
        panic!("TlsConfig::with_native_roots() requires feature: native-roots");
    }
}

#[cfg(feature = "native-roots")]
impl TlsConfig {
    pub fn with_native_roots() -> TlsConfig {
        TlsConfig {
            root_certs: load_root_certs(),
            ..Default::default()
        }
    }
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            client_cert: None,
            root_certs: vec![],
            use_sni: true,
            disable_verification: false,
        }
    }
}
