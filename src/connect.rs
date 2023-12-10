use std::fmt;
use std::io::Result as IoResult;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// A custom Connector to override the default TcpStream connector.
pub trait TcpConnector: Send + Sync {
    fn connect(&self, addr: &SocketAddr) -> IoResult<TcpStream> {
        TcpStream::connect(addr)
    }

    fn connect_timeout(&self, addr: &SocketAddr, timeout: Duration) -> IoResult<TcpStream> {
        TcpStream::connect_timeout(addr, timeout)
    }
}

#[derive(Debug)]
pub(crate) struct StdTcpConnector;

impl TcpConnector for StdTcpConnector {}

#[derive(Clone)]
pub(crate) struct ArcTcpConnector(Arc<dyn TcpConnector>);

impl<R> From<R> for ArcTcpConnector
where
    R: TcpConnector + 'static,
{
    fn from(r: R) -> Self {
        Self(Arc::new(r))
    }
}

impl fmt::Debug for ArcTcpConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArcTcpConnector(...)")
    }
}

impl std::ops::Deref for ArcTcpConnector {
    type Target = dyn TcpConnector;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}
