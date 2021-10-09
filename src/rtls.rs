use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use crate::ErrorKind;
use crate::{
    stream::{HttpsStream, TlsConnector},
    Error,
};
use rustls::Session;

#[allow(deprecated)]
fn is_close_notify(e: &std::io::Error) -> bool {
    if e.kind() != io::ErrorKind::ConnectionAborted {
        return false;
    }

    if let Some(msg) = e.get_ref() {
        // :(

        return msg.description().contains("CloseNotify");
    }

    false
}

struct RustlsStream(rustls::StreamOwned<rustls::ClientSession, TcpStream>);

impl HttpsStream for RustlsStream {
    fn socket(&self) -> Option<&TcpStream> {
        Some(self.0.get_ref())
    }
}

// TODO: After upgrading to rustls 0.20 or higher, we can remove these Read
// and Write impls, leaving only `impl TlsStream for rustls::StreamOwned...`.
// Currently we need to implement Read in order to treat close_notify specially.
// The next release of rustls will handle close_notify in a more intuitive way.
impl Read for RustlsStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            Ok(size) => Ok(size),
            Err(ref e) if is_close_notify(e) => Ok(0),
            Err(e) => Err(e),
        }
    }
}

impl Write for RustlsStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(feature = "native-certs")]
fn configure_certs(config: &mut rustls::ClientConfig) {
    config.root_store =
        rustls_native_certs::load_native_certs().expect("Could not load platform certs");
}

#[cfg(not(feature = "native-certs"))]
fn configure_certs(config: &mut rustls::ClientConfig) {
    config
        .root_store
        .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
}

impl TlsConnector for Arc<rustls::ClientConfig> {
    fn connect(
        &self,
        dns_name: &str,
        mut tcp_stream: TcpStream,
    ) -> Result<Box<dyn HttpsStream>, Error> {
        let sni = webpki::DNSNameRef::try_from_ascii_str(dns_name)
            .map_err(|err| ErrorKind::Dns.new().src(err))?;
        let mut sess = rustls::ClientSession::new(self, sni);

        sess.complete_io(&mut tcp_stream)
            .map_err(|err| ErrorKind::ConnectionFailed.new().src(err))?;
        let stream = rustls::StreamOwned::new(sess, tcp_stream);

        Ok(Box::new(RustlsStream(stream)))
    }
}

pub fn default_tls_config() -> Arc<dyn TlsConnector> {
    use once_cell::sync::Lazy;
    static TLS_CONF: Lazy<Arc<dyn TlsConnector>> = Lazy::new(|| {
        let mut config = rustls::ClientConfig::new();
        configure_certs(&mut config);
        Arc::new(Arc::new(config))
    });
    TLS_CONF.clone()
}
