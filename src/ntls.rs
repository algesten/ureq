use crate::error::Error;
use crate::error::ErrorKind;
use crate::stream::{HttpsStream, TlsConnector};

use std::net::TcpStream;
use std::sync::Arc;

#[allow(dead_code)]
pub(crate) fn default_tls_config() -> std::sync::Arc<dyn TlsConnector> {
    Arc::new(native_tls::TlsConnector::new().unwrap())
}

impl TlsConnector for native_tls::TlsConnector {
    fn connect(
        &self,
        dns_name: &str,
        tcp_stream: TcpStream,
    ) -> Result<Box<dyn HttpsStream>, Error> {
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
impl HttpsStream for native_tls::TlsStream<TcpStream> {
    fn socket(&self) -> Option<&TcpStream> {
        Some(self.get_ref())
    }
}
