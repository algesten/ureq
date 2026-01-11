use std::convert::TryFrom;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, OnceLock};
use std::{fmt, io};

use crate::tls::{RootCerts, TlsProvider};
use crate::{transport::time::Duration, transport::*, Error};
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

        let connector = self.get_cached_native_tls_connector(details)?;

        let domain = details
            .uri
            .authority()
            .expect("uri authority for tls")
            .host()
            .to_string();

        let adapter = ErrorCapture::wrap(TransportAdapter::new(transport.boxed()));
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
    fn get_cached_native_tls_connector(
        &self,
        details: &ConnectionDetails,
    ) -> Result<Arc<TlsConnector>, Error> {
        let tls_config = details.config.tls_config();

        let connector = if details.request_level {
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
            #[cfg(feature = "native-tls-webpki-roots")]
            RootCerts::WebPki => {
                // Only use the specific roots.
                builder.disable_built_in_roots(true);
                let certs = webpki_root_certs::TLS_SERVER_ROOT_CERTS
                    .iter()
                    .map(|c| c.as_ref());
                add_valid_der(certs, &mut builder);
            }
            #[cfg(not(feature = "native-tls-webpki-roots"))]
            RootCerts::WebPki => {
                panic!("WebPki is disabled. You need to explicitly configure root certs on Agent");
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
        let stream = self.stream.handshaken(timeout)?;
        stream.get_mut().get_mut().set_timeout(timeout);

        let output = &self.buffers.output()[..amount];
        let ret = stream.write_all(output);

        // Surface errors capture below NativeTls primarily.
        stream.get_mut().take_captured()?;

        // Then NativeTls errors
        ret?;

        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        let stream = self.stream.handshaken(timeout)?;
        stream.get_mut().get_mut().set_timeout(timeout);

        let input = self.buffers.input_append_buf();
        let result = stream.read(input);

        let amount = match result {
            Ok(v) => {
                if v == 0 {
                    // NativeTls normalizes some error conditions to Ok(0)
                    stream.get_mut().take_captured()?;
                }

                v
            }
            Err(e) => {
                // First captured
                stream.get_mut().take_captured()?;

                // Then NativeTls
                return Err(e.into());
            }
        };

        self.buffers.input_appended(amount);

        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        let timeout = NextTimeout {
            after: Duration::Exact(std::time::Duration::from_secs(1)),
            reason: crate::Timeout::Global,
        };

        self.stream
            .handshaken(timeout)
            .map(|c| c.get_mut().get_mut().get_mut().is_open())
            .unwrap_or(false)
    }

    fn is_tls(&self) -> bool {
        true
    }
}

/// Helper to delay the handshake until we are starting IO.
/// This normalizes native-tls to behave like rustls.
enum LazyStream {
    Unstarted(Option<(Arc<TlsConnector>, String, ErrorCapture<TransportAdapter>)>),
    Started(TlsStream<ErrorCapture<TransportAdapter>>),
}

impl LazyStream {
    fn handshaken(
        &mut self,
        timeout: NextTimeout,
    ) -> Result<&mut TlsStream<ErrorCapture<TransportAdapter>>, Error> {
        match self {
            LazyStream::Unstarted(v) => {
                let (conn, domain, mut adapter) = v.take().unwrap();

                // Respect timeout during TLS handshake
                adapter.get_mut().set_timeout(timeout);
                let capture = adapter.capture();

                let result = conn.connect(&domain, adapter).map_err(|e| match e {
                    HandshakeError::Failure(e) => e,
                    HandshakeError::WouldBlock(_) => unreachable!(),
                });

                let stream = match result {
                    Ok(v) => v,
                    Err(e) => {
                        // The error might originate in a Error::Timeout in the underlying adapter.
                        // If so, we receive that error in this mpsc::Receiver. That's a more specific
                        // error than the NativeTls::Error type.
                        let mut lock = capture.lock().unwrap();
                        if let Some(error) = lock.take() {
                            return Err(error);
                        }

                        return Err(e.into());
                    }
                };

                *self = LazyStream::Started(stream);
                // Next time we hit the other match arm
                self.handshaken(timeout)
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

/// A wrapper that captures and preserves underlying transport errors.
///
/// Native-tls may normalize or obscure specific errors from the underlying transport,
/// such as timeout errors. This wrapper intercepts and stores ureq errors for later
/// retrieval, allowing us to surface more specific error information (like timeouts)
/// rather than generic TLS errors.
///
/// When an error occurs during IO operations, it's captured in the mutex and a generic
/// "fake error" is returned to native-tls.
struct ErrorCapture<S> {
    stream: S,
    capture: Arc<Mutex<Option<Error>>>,
}

impl<S: Read + Write> ErrorCapture<S> {
    fn wrap(stream: S) -> Self {
        ErrorCapture {
            stream,
            capture: Arc::new(Mutex::new(None)),
        }
    }

    fn capture(&self) -> Arc<Mutex<Option<Error>>> {
        self.capture.clone()
    }

    fn get_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    fn take_captured(&self) -> Result<(), Error> {
        let mut lock = self.capture.lock().unwrap();
        if let Some(error) = lock.take() {
            return Err(error);
        }
        Ok(())
    }
}

impl<S: Read> Read for ErrorCapture<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream
            .read(buf)
            .map_err(|e| capture_error(e, &self.capture))
    }
}

impl<S: Write> Write for ErrorCapture<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream
            .write(buf)
            .map_err(|e| capture_error(e, &self.capture))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream
            .flush()
            .map_err(|e| capture_error(e, &self.capture))
    }
}

fn capture_error(e: io::Error, capture: &Arc<Mutex<Option<Error>>>) -> io::Error {
    let error: Error = e.into();

    let mut lock = capture.lock().unwrap();
    *lock = Some(error);

    io::Error::new(io::ErrorKind::Other, "fake error towards native-tls")
}
