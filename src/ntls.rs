use crate::error::Error;
use crate::error::ErrorKind;
use crate::stream::{ReadWrite, TlsConnector};

use std::net::TcpStream;
use std::sync::Arc;

#[allow(dead_code)]
pub(crate) fn default_tls_config() -> std::sync::Arc<dyn TlsConnector> {
    Arc::new(native_tls::TlsConnector::new().unwrap())
}

impl TlsConnector for native_tls::TlsConnector {
    fn connect(&self, dns_name: &str, tcp_stream: TcpStream) -> Result<Box<dyn ReadWrite>, Error> {
        let stream =
            native_tls::TlsConnector::connect(self, dns_name, tcp_stream).map_err(|e| {
                ErrorKind::ConnectionFailed
                    .msg("native_tls connect failed")
                    .src(e)
            })?;

        Ok(Box::new(stream))
    }
}

#[cfg(feature = "native-tls")]
impl ReadWrite for native_tls::TlsStream<TcpStream> {
    fn socket(&self) -> Option<&TcpStream> {
        Some(self.get_ref())
    }
}

#[cfg(feature = "native-tls")]
impl fmt::Debug for native_tls::TlsStream<TcpStream> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("native_tls::TlsStream")
            .field("socket", self.get_ref())
            .finish()
    }
}
