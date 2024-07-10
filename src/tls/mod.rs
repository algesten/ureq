mod cert;
pub use cert::{parse_pem, Certificate, PemItem, PrivateKey};

#[cfg(feature = "rustls")]
mod rustls;

pub struct TlsConfig<'a> {
    pub client_cert: Option<(Certificate<'a>, PrivateKey<'a>)>,
    pub root_certs: &'a [Certificate<'a>],
    pub use_sni: bool,
    pub disable_certificate_verification: bool,
}
