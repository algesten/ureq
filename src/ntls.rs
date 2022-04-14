use crate::error::Error;
use crate::error::ErrorKind;
use crate::stream::{ReadWrite, Stream, TlsConnector};

use std::net::TcpStream;
use std::sync::Arc;

#[allow(dead_code)]
pub(crate) fn default_tls_config() -> std::sync::Arc<dyn TlsConnector> {
    Arc::new(native_tls::TlsConnector::new().unwrap())
}

impl TlsConnector for native_tls::TlsConnector {
    fn connect(&self, dns_name: &str, tcp_stream: Stream) -> Result<Box<dyn ReadWrite>, Error> {
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
impl ReadWrite for native_tls::TlsStream<Stream> {
    fn socket(&self) -> Option<&TcpStream> {
        self.get_ref().socket()
    }
}
