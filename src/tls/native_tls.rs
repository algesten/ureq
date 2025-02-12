use std::convert::TryFrom;
use std::fmt;
use std::io::{Read, Write};
use std::sync::{Arc, OnceLock};

use crate::config::Config;
use crate::tls::{RootCerts, TlsProvider};
use crate::{transport::*, Error};
use der::pem::LineEnding;
use der::Document;
use native_tls::{Certificate, HandshakeError, Identity, TlsConnector};
use native_tls::{TlsConnectorBuilder, TlsStream};

use super::TlsConfig;

/// Wrapper for TLS using native-tls.
///
/// Requires feature flag **native-tls**.
#[derive(Default)]
pub struct NativeTlsConnector {
    connector: OnceLock<CachedNativeTlsConnector>,
}

struct CachedNativeTlsConnector {
    config_hash: u64,
    native_tls_connector: Arc<TlsConnector>,
}

impl<In: Transport> Connector<In> for NativeTlsConnector {
    type Out = Either<In, NativeTlsTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        let Some(transport) = chained else {
            panic!("NativeTlsConnector requires a chained transport");
        };

        // Only add TLS if we are connecting via HTTPS and the transport isn't TLS
        // already, otherwise use chained transport as is.
        if !details.needs_tls() || transport.is_tls() {
            trace!("Skip");
            return Ok(Some(Either::A(transport)));
        }

        if details.config.tls_config().provider != TlsProvider::NativeTls {
            debug!("Skip because config is not set to Native TLS");
            return Ok(Some(Either::A(transport)));
        }

        trace!("Try wrap TLS");

        let connector = self.get_cached_native_tls_connector(details.config)?;

        let domain = details
            .uri
            .authority()
            .expect("uri authority for tls")
            .host()
            .to_string();

        let adapter = TransportAdapter::new(transport.boxed());
        let stream = LazyStream::Unstarted(Some((connector, domain, adapter)));

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );

        let transport = NativeTlsTransport { buffers, stream };

        debug!("Wrapped TLS");

        Ok(Some(Either::B(transport)))
    }
}

impl NativeTlsConnector {
    fn get_cached_native_tls_connector(&self, config: &Config) -> Result<Arc<TlsConnector>, Error> {
        let tls_config = config.tls_config();

        let connector = if config.request_level {
            // If the TlsConfig is request level, it is not allowed to
            // initialize the self.config OnceLock, but it should
            // reuse the cached value if it is the same TlsConfig
            // by comparing the config_hash value.

            let is_cached = self
                .connector
                .get()
                .map(|c| c.config_hash == tls_config.hash_value())
                .unwrap_or(false);

            if is_cached {
                // unwrap is ok because if is_cached is true we must have had a value.
                self.connector.get().unwrap().native_tls_connector.clone()
            } else {
                build_connector(tls_config)?.native_tls_connector
            }
        } else {
            // Initialize the connector on first run.
            let connector_ref = match self.connector.get() {
                Some(v) => v,
                None => {
                    // This is unlikely to be racy, but if it is, doesn't matter much.
                    let c = build_connector(tls_config)?;
                    // Maybe someone else set it first. Weird, but ok.
                    let _ = self.connector.set(c);
                    self.connector.get().unwrap()
                }
            };

            connector_ref.native_tls_connector.clone() // cheap clone due to Arc
        };

        Ok(connector)
    }
}

fn build_connector(tls_config: &TlsConfig) -> Result<CachedNativeTlsConnector, Error> {
    let mut builder = TlsConnector::builder();

    if tls_config.disable_verification {
        debug!("Certificate verification disabled");
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    } else {
        match &tls_config.root_certs {
            RootCerts::Specific(certs) => {
                // Only use the specific roots.
                builder.disable_built_in_roots(true);
                add_valid_der(certs.iter().map(|c| c.der()), &mut builder);
            }
            RootCerts::PlatformVerifier => {
                // We only use the built-in roots.
                builder.disable_built_in_roots(false);
            }
            RootCerts::WebPki => {
                // Only use the specific roots.
                builder.disable_built_in_roots(true);
                let certs = webpki_root_certs::TLS_SERVER_ROOT_CERTS
                    .iter()
                    .map(|c| c.as_ref());
                add_valid_der(certs, &mut builder);
            }
        }
    }

    if let Some(certs_and_key) = &tls_config.client_cert {
        let (certs, key) = &*certs_and_key.0;
        let certs_pem = certs
            .iter()
            .map(|c| pemify(c.der(), "CERTIFICATE"))
            .collect::<Result<String, Error>>()?;

        let key_pem = pemify(key.der(), "PRIVATE KEY")?;

        debug!("Use client certficiate with key kind {:?}", key.kind());

        let identity = Identity::from_pkcs8(certs_pem.as_bytes(), key_pem.as_bytes())?;
        builder.identity(identity);
    }

    builder.use_sni(tls_config.use_sni);

    if !tls_config.use_sni {
        debug!("Disable SNI");
    }

    let conn = builder.build()?;

    let cached = CachedNativeTlsConnector {
        config_hash: tls_config.hash_value(),
        native_tls_connector: Arc::new(conn),
    };

    Ok(cached)
}

fn add_valid_der<'a, C>(certs: C, builder: &mut TlsConnectorBuilder)
where
    C: Iterator<Item = &'a [u8]>,
{
    let mut added = 0;
    let mut ignored = 0;
    for der in certs {
        let c = match Certificate::from_der(der) {
            Ok(v) => v,
            Err(e) => {
                // Invalid/expired/broken root certs are expected
                // in a native root store.
                trace!("Ignore invalid root cert: {}", e);
                ignored += 1;
                continue;
            }
        };
        builder.add_root_certificate(c);
        added += 1;
    }
    debug!("Added {} and ignored {} root certs", added, ignored);
}

fn pemify(der: &[u8], label: &'static str) -> Result<String, Error> {
    let doc = Document::try_from(der)?;
    let pem = doc.to_pem(label, LineEnding::LF)?;
    Ok(pem)
}

pub struct NativeTlsTransport {
    buffers: LazyBuffers,
    stream: LazyStream,
}

impl Transport for NativeTlsTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        let stream = self.stream.handshaken()?;
        stream.get_mut().set_timeout(timeout);

        let output = &self.buffers.output()[..amount];
        stream.write_all(output)?;

        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        if self.buffers.can_use_input() {
            return Ok(true);
        }

        let stream = self.stream.handshaken()?;
        stream.get_mut().set_timeout(timeout);

        let input = self.buffers.input_append_buf();
        let amount = stream.read(input)?;
        self.buffers.input_appended(amount);

        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        self.stream
            .handshaken()
            .map(|c| c.get_mut().get_mut().is_open())
            .unwrap_or(false)
    }

    fn is_tls(&self) -> bool {
        true
    }
}

/// Helper to delay the handshake until we are starting IO.
/// This normalizes native-tls to behave like rustls.
enum LazyStream {
    Unstarted(Option<(Arc<TlsConnector>, String, TransportAdapter)>),
    Started(TlsStream<TransportAdapter>),
}

impl LazyStream {
    fn handshaken(&mut self) -> Result<&mut TlsStream<TransportAdapter>, Error> {
        match self {
            LazyStream::Unstarted(v) => {
                let (conn, domain, adapter) = v.take().unwrap();
                let stream = conn.connect(&domain, adapter).map_err(|e| match e {
                    HandshakeError::Failure(e) => e,
                    HandshakeError::WouldBlock(_) => unreachable!(),
                })?;
                *self = LazyStream::Started(stream);
                // Next time we hit the other match arm
                self.handshaken()
            }
            LazyStream::Started(v) => Ok(v),
        }
    }
}
impl fmt::Debug for NativeTlsConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeTlsConnector").finish()
    }
}

impl fmt::Debug for NativeTlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeTlsTransport").finish()
    }
}
