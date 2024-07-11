use std::convert::TryInto;
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use http::uri::Scheme;
use once_cell::sync::OnceCell;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, ALL_VERSIONS};
use rustls_pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer,
    ServerName,
};

use crate::time::Duration;
use crate::tls::cert::KeyKind;
use crate::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, Transport, TransportAdapter,
};
use crate::Error;

use super::TlsConfig;

#[derive(Default)]
pub struct RustlsConnector {
    config: OnceCell<Arc<ClientConfig>>,
}

impl Connector for RustlsConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        let transport = match chained {
            Some(v) => v,
            None => panic!("RustlConnector requires a chained transport"),
        };

        // Only add TLS if we are connecting via HTTPS and the transport isn't TLS
        // already, otherwise use chained transport as is.
        if details.uri.scheme() != Some(&Scheme::HTTPS) || transport.is_tls() {
            trace!("Skip");
            return Ok(Some(transport));
        }

        trace!("Try wrap in TLS");

        let tls_config = &details.config.tls_config;

        // Initialize the config on first run.
        let config_ref = self.config.get_or_init(|| build_config(tls_config));
        let config = config_ref.clone(); // cheap clone due to Arc

        let name_borrowed: ServerName<'_> = details
            .uri
            .authority()
            .expect("uri authority for tls")
            .host()
            .try_into()
            .map_err(|_| Error::Other("rustls invalid dns name error"))?;

        let name = name_borrowed.to_owned();

        let conn = ClientConnection::new(config, name)?;
        let stream = StreamOwned {
            conn,
            sock: TransportAdapter::new(transport),
        };

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size,
            details.config.output_buffer_size,
        );

        let transport = Box::new(RustlsTransport { buffers, stream });

        debug!("Wrapped TLS");

        Ok(Some(transport))
    }
}

fn build_config(tls_config: &TlsConfig) -> Arc<ClientConfig> {
    // Improve chances of ureq working out-of-the-box by not requiring the user
    // to select a default crypto provider.
    let provider = Arc::new(rustls::crypto::ring::default_provider());

    let builder = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(ALL_VERSIONS)
        .expect("all TLS versions");

    let builder = if tls_config.disable_verification {
        debug!("Certificate verification disabled");
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DisabledVerifier))
    } else {
        let root_certs = tls_config
            .root_certs
            .iter()
            .map(|c| CertificateDer::from(c.der()));
        let mut root_store = RootCertStore::empty();
        let (added, ignored) = root_store.add_parsable_certificates(root_certs);
        debug!("Added {} and ignored {} root certs", added, ignored);

        builder.with_root_certificates(root_store)
    };

    let mut config = if let Some((certs, key)) = &tls_config.client_cert {
        let cert_chain = certs
            .iter()
            .map(|c| CertificateDer::from(c.der()).into_owned());

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

    Arc::new(config)
}

struct RustlsTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, TransportAdapter>,
}

impl Transport for RustlsTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error> {
        self.stream.get_mut().timeout = timeout;

        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output)?;

        Ok(())
    }

    fn await_input(&mut self, timeout: Duration) -> Result<(), Error> {
        if self.buffers.can_use_input() {
            return Ok(());
        }

        self.stream.get_mut().timeout = timeout;

        let input = self.buffers.input_mut();
        let amount = self.stream.read(input)?;
        self.buffers.add_filled(amount);

        Ok(())
    }

    fn consume_input(&mut self, amount: usize) {
        self.buffers.consume(amount);
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
        todo!()
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
            .field("chained", &self.stream.sock.transport)
            .finish()
    }
}
