use std::convert::TryInto;
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rustls::client::ResolvesClientCert;
use rustls::sign::{CertifiedKey, Signer, SigningKey};
use rustls::{ClientConfig, ClientConnection, SignatureAlgorithm, StreamOwned};
use rustls_pki_types::ServerName;

use rustls_platform_verifier::BuilderVerifierExt;
use ureq::config::Config;
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectProxyConnector, ConnectionDetails, Connector, LazyBuffers, TcpConnector,
};
use ureq::unversioned::transport::{Either, NextTimeout, Transport, TransportAdapter};
use ureq::{Agent, Error};

use log::*;

pub fn main() {
    // Key and certificate working against a TPM module.
    let certified = CertifiedKey::new(vec![], Arc::new(TpmKey));

    // Resolver to select the correct TPM cert/key
    let my_resolver = MyResolver(Arc::new(certified));

    // Bespoke rustls config
    let config = ClientConfig::builder()
        .with_platform_verifier()
        .with_client_cert_resolver(Arc::new(my_resolver));

    // Connector for using bespoke rustls
    let connector = CustomRustlsConnector::new(Arc::new(config));

    // Chain that allows a CONNECT proxy or regular Tcp followed by our bespoke rustls.
    let chain =
        ().chain(ConnectProxyConnector::default())
            .chain(TcpConnector::default())
            .chain(connector);

    // Agent with this connector chain
    let agent = Agent::with_parts(Config::default(), chain, DefaultResolver::default());

    let _response = agent.get("https://my-special-server.com").call().unwrap();
}

#[derive(Debug)]
struct TpmKey;

impl SigningKey for TpmKey {
    fn choose_scheme(&self, _offered: &[rustls::SignatureScheme]) -> Option<Box<dyn Signer>> {
        todo!()
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        todo!()
    }
}

#[derive(Debug)]
struct MyResolver(Arc<CertifiedKey>);

impl ResolvesClientCert for MyResolver {
    fn resolve(
        &self,
        _root_hint_subjects: &[&[u8]],
        _sigschemes: &[rustls::SignatureScheme],
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }

    fn has_certs(&self) -> bool {
        todo!()
    }
}

/// Bespoke TLS connector using Rustls.
pub struct CustomRustlsConnector {
    config: Arc<ClientConfig>,
}

impl CustomRustlsConnector {
    pub fn new(config: Arc<ClientConfig>) -> Self {
        CustomRustlsConnector { config }
    }
}

impl<In: Transport> Connector<In> for CustomRustlsConnector {
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

        trace!("Try wrap in TLS");

        let config = self.config.clone();

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

impl fmt::Debug for CustomRustlsConnector {
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
