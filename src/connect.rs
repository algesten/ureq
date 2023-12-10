use std::fmt;
use std::io::Result as IoResult;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// A custom Connector to override the default TcpStream connector.
pub trait Connector: Send + Sync {
    fn connect(&self, addr: &SocketAddr) -> IoResult<TcpStream> {
        TcpStream::connect(addr)
    }

    fn connect_timeout(&self, addr: &SocketAddr, timeout: Duration) -> IoResult<TcpStream> {
        TcpStream::connect_timeout(addr, timeout)
    }
}

#[derive(Debug)]
pub(crate) struct StdTcpConnector;

impl Connector for StdTcpConnector {}

#[derive(Clone)]
pub(crate) struct ArcConnector(Arc<dyn Connector>);

impl<R> From<R> for ArcConnector
where
    R: Connector + 'static,
{
    fn from(r: R) -> Self {
        Self(Arc::new(r))
    }
}

impl fmt::Debug for ArcConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArcConnector(...)")
    }
}

impl std::ops::Deref for ArcConnector {
    type Target = dyn Connector;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}
