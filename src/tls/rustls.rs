use std::convert::TryInto;
use std::fmt;
use std::io::{Read, Write};
use std::sync::{Arc, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, ALL_VERSIONS};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer};
use rustls_pki_types::{PrivateSec1KeyDer, ServerName};

use crate::tls::cert::KeyKind;
use crate::tls::{RootCerts, TlsProvider};
use crate::transport::{Buffers, ConnectionDetails, Connector, LazyBuffers};
use crate::transport::{Either, NextTimeout, Transport, TransportAdapter};
use crate::Error;

use super::TlsConfig;

/// Wrapper for TLS using rustls.
///
/// Requires feature flag **rustls**.
#[derive(Default)]
pub struct RustlsConnector {
    config: OnceLock<CachedRustlConfig>,
}

struct CachedRustlConfig {
    config_hash: u64,
    rustls_config: Arc<ClientConfig>,
}

impl<In: Transport> Connector<In> for RustlsConnector {
    type Out = Either<In, RustlsTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        let Some(transport) = chained else {
            panic!("RustlConnector requires a chained transport");
        };

        // Only add TLS if we are connecting via HTTPS and the transport isn't TLS
        // already, otherwise use chained transport as is.
        if !details.needs_tls() || transport.is_tls() {
            trace!("Skip");
            return Ok(Some(Either::A(transport)));
        }

        if details.config.tls_config().provider != TlsProvider::Rustls {
            debug!("Skip because config is not set to Rustls");
            return Ok(Some(Either::A(transport)));
        }

        trace!("Try wrap in TLS");

        let config = self.get_cached_config(details)?;

        let name_borrowed: ServerName<'_> = details
            .uri
            .authority()
            .expect("uri authority for tls")
            .host()
            .try_into()
            .map_err(|e| {
                debug!("rustls invalid dns name: {}", e);
                Error::Tls("Rustls invalid dns name error")
            })?;

        let name = name_borrowed.to_owned();

        let conn = ClientConnection::new(config, name)?;
        let stream = StreamOwned {
            conn,
            sock: TransportAdapter::new(transport.boxed()),
        };

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );

        let transport = RustlsTransport { buffers, stream };

        debug!("Wrapped TLS");

        Ok(Some(Either::B(transport)))
    }
}

impl RustlsConnector {
    fn get_cached_config(&self, details: &ConnectionDetails) -> Result<Arc<ClientConfig>, Error> {
        let tls_config = details.config.tls_config();

        Ok(if details.request_level {
            // If the TlsConfig is request level, it is not allowed to
            // initialize the self.config OnceLock, but it should
            // reuse the cached value if it is the same TlsConfig
            // by comparing the config_hash value.

            let is_cached = self
                .config
                .get()
                .map(|c| c.config_hash == tls_config.hash_value())
                .unwrap_or(false);

            if is_cached {
                // unwrap is ok because if is_cached is true we must have had a value.
                self.config.get().unwrap().rustls_config.clone()
            } else {
                build_config(tls_config)?.rustls_config
            }
        } else {
            // On agent level, we initialize the config on first run. This is
            // the value we want to cache.
            //
            // NB: This init is a racey. The problem is that build_config() must
            //     return a Result, and OnceLock::get_or_try_init is not stabilized
            //     https://github.com/rust-lang/rust/issues/109737
            //     In case we're slamming a newly created Agent with many simultaneous
            //     TLS requests, this might create some unnecessary/discarded rustls configs.
            loop {
                if let Some(config_ref) = self.config.get() {
                    break config_ref.rustls_config.clone();
                } else {
                    let config = build_config(tls_config)?;
                    let _ = self.config.set(config);
                }
            }
        })
    }
}

fn build_config(tls_config: &TlsConfig) -> Result<CachedRustlConfig, Error> {
    // 1. Prefer provider set by TlsConfig.
    // 2. Use process wide default set in rustls library.
    // 3. Pick ring, if it is enabled (the default behavior).
    // 4. Error (never pick up a default from feature flags alone).
    let provider = tls_config
        .rustls_crypto_provider
        .clone()
        .or(rustls::crypto::CryptoProvider::get_default().cloned())
        .unwrap_or_else(ring_if_enabled);

    #[cfg(feature = "_ring")]
    fn ring_if_enabled() -> Arc<CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    #[cfg(not(feature = "_ring"))]
    fn ring_if_enabled() -> Arc<CryptoProvider> {
        panic!(
            "No CryptoProvider for Rustls. Either enable feature `rustls`, or set process
            default using CryptoProvider::set_default(), or configure
            TlsConfig::rustls_crypto_provider()"
        );
    }

    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(ALL_VERSIONS)
        .expect("all TLS versions");

    let builder = if tls_config.disable_verification {
        debug!("Certificate verification disabled");
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DisabledVerifier))
    } else {
        match &tls_config.root_certs {
            RootCerts::Specific(certs) => {
                let root_certs = certs.iter().map(|c| CertificateDer::from(c.der()));

                let mut root_store = RootCertStore::empty();
                let (added, ignored) = root_store.add_parsable_certificates(root_certs);
                debug!("Added {} and ignored {} root certs", added, ignored);

                builder.with_root_certificates(root_store)
            }
            #[cfg(not(feature = "platform-verifier"))]
            RootCerts::PlatformVerifier => {
                panic!("Rustls + PlatformVerifier requires feature: platform-verifier");
            }
            #[cfg(feature = "platform-verifier")]
            RootCerts::PlatformVerifier => builder
                // This actually not dangerous. The rustls_platform_verifier is safe.
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(
                    rustls_platform_verifier::Verifier::new(provider)?,
                )),
            #[cfg(feature = "rustls-webpki-roots")]
            RootCerts::WebPki => {
                let root_store = RootCertStore {
                    roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                };
                builder.with_root_certificates(root_store)
            }
            #[cfg(not(feature = "rustls-webpki-roots"))]
            RootCerts::WebPki => {
                panic!("WebPki is disabled. You need to explicitly configure root certs on Agent");
            }
        }
    };

    let mut config = if let Some(certs_and_key) = &tls_config.client_cert {
        let cert_chain = certs_and_key
            .certs()
            .iter()
            .map(|c| CertificateDer::from(c.der()).into_owned());

        let key = certs_and_key.private_key();

        let key_der = match key.kind() {
            KeyKind::Pkcs1 => PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(key.der())),
            KeyKind::Pkcs8 => PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.der())),
            KeyKind::Sec1 => PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(key.der())),
        }
        .clone_key();
        debug!("Use client certficiate with key kind {:?}", key.kind());

        builder
            .with_client_auth_cert(cert_chain.collect(), key_der)
            .expect("valid client auth certificate")
    } else {
        builder.with_no_client_auth()
    };

    config.enable_sni = tls_config.use_sni;

    if !tls_config.use_sni {
        debug!("Disable SNI");
    }

    Ok(CachedRustlConfig {
        config_hash: tls_config.hash_value(),
        rustls_config: Arc::new(config),
    })
}

pub struct RustlsTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, TransportAdapter>,
}

impl Transport for RustlsTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        self.stream.get_mut().set_timeout(timeout);

        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output)?;

        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        self.stream.get_mut().set_timeout(timeout);

        let input = self.buffers.input_append_buf();
        let amount = self.stream.read(input)?;
        self.buffers.input_appended(amount);

        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        self.stream.get_mut().get_mut().is_open()
    }

    fn is_tls(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct DisabledVerifier;

impl ServerCertVerifier for DisabledVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA1,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}

impl fmt::Debug for RustlsConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustlsConnector").finish()
    }
}

impl fmt::Debug for RustlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustlsTransport")
            .field("chained", &self.stream.sock.inner())
            .finish()
    }
}
