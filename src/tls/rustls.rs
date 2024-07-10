use std::convert::TryInto;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, io};

use http::uri::Scheme;
use once_cell::sync::OnceCell;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer,
    ServerName,
};

use crate::time::Instant;
use crate::tls::cert::KeyKind;
use crate::transport::{Buffers, ConnectionDetails, Connector, LazyBuffers, Transport};
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

        // Only add TLS if we are connecting via HTTPS, otherwise
        // use chained transport as is.
        if details.uri.scheme() != Some(&Scheme::HTTPS) {
            return Ok(Some(transport));
        }

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
            sock: StreamAdapter {
                timeout: Instant::NotHappening.duration_since(Instant::now()),
                transport,
            },
        };

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size,
            details.config.output_buffer_size,
        );

        let transport = Box::new(RustlsTransport { buffers, stream });

        Ok(Some(transport))
    }
}

fn build_config(tls_config: &TlsConfig) -> Arc<ClientConfig> {
    let root_certs = tls_config
        .root_certs
        .iter()
        .map(|c| CertificateDer::from(c.der()));
    let mut root_store = RootCertStore::empty();
    root_store.add_parsable_certificates(root_certs);

    let builder = if tls_config.disable_verification {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DisabledVerifier))
    } else {
        ClientConfig::builder().with_root_certificates(root_store)
    };

    let config = if let Some((certs, key)) = &tls_config.client_cert {
        let cert_chain = certs
            .iter()
            .map(|c| CertificateDer::from(c.der()).into_owned());

        let key_der = match key.kind() {
            KeyKind::Pkcs1 => PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(key.der())),
            KeyKind::Pkcs8 => PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.der())),
            KeyKind::Sec1 => PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(key.der())),
        }
        .clone_key();

        builder
            .with_client_auth_cert(cert_chain.collect(), key_der)
            .expect("valid client auth certificate")
    } else {
        builder.with_no_client_auth()
    };

    Arc::new(config)
}

struct RustlsTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, StreamAdapter>,
}

impl Transport for RustlsTransport {
    fn borrow_buffers(&mut self, input_as_tmp: bool) -> Buffers {
        self.buffers.borrow_mut(input_as_tmp)
    }

    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error> {
        let buffers = self.buffers.borrow_mut(false);
        self.stream.sock.timeout = timeout;
        self.stream.write_all(&buffers.output[..amount])?;
        Ok(())
    }

    fn await_input(&mut self, timeout: Duration) -> Result<Buffers, Error> {
        if self.buffers.unconsumed() > 0 {
            return Ok(self.buffers.borrow_mut(false));
        }

        // Ensure we get the entire input buffer to write to.
        self.buffers.assert_and_clear_input_filled();

        // Read more
        self.stream.sock.timeout = timeout;
        let buffers = self.buffers.borrow_mut(false);
        let amount = self.stream.read(buffers.input)?;

        // Cap the input
        self.buffers.set_input_filled(amount);

        Ok(self.buffers.borrow_mut(false))
    }

    fn consume_input(&mut self, amount: usize) {
        self.buffers.consume_input(amount)
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

pub struct StreamAdapter {
    pub timeout: Duration,
    pub transport: Box<dyn Transport>,
}

impl io::Read for StreamAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let buffers = self
            .transport
            .await_input(self.timeout)
            .map_err(|e| e.into_io())?;

        let max = buf.len().min(buffers.input.len());
        buf[..max].copy_from_slice(&buffers.input[..max]);
        self.transport.consume_input(max);

        Ok(max)
    }
}

impl io::Write for StreamAdapter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let buffers = self.transport.borrow_buffers(false);

        let max = buf.len().min(buffers.output.len());
        buffers.output[..max].copy_from_slice(&buf[..max]);
        self.transport
            .transmit_output(max, self.timeout)
            .map_err(|e| e.into_io())?;

        Ok(max)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
