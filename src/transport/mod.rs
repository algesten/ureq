use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::Duration;

use http::Uri;

use crate::proxy::Proxy;
use crate::resolver::Resolver;
use crate::{AgentConfig, Error};

#[cfg(feature = "rustls")]
use crate::tls::RustlsConnector;

use self::tcp::TcpConnector;

mod lazybuf;
pub use lazybuf::LazyBuffers;

mod tcp;

mod io;
pub use io::TransportAdapter;

pub trait Connector: Debug + Send + Sync + 'static {
    fn boxed(self) -> Box<dyn Connector>
    where
        Self: Sized,
    {
        Box::new(self)
    }

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error>;
}

pub struct ConnectionDetails<'a> {
    pub uri: &'a Uri,
    pub addr: SocketAddr,
    pub proxy: &'a Option<Proxy>,
    pub resolver: &'a dyn Resolver,
    pub config: &'a AgentConfig,

    // TODO(martin): Make mechanism to lower duration for each step in the connector chain.
    pub timeout: Duration,
}

pub trait Transport: Debug + Send + Sync {
    fn borrow_buffers(&mut self, input_as_tmp: bool) -> Buffers;
    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error>;
    fn await_input(&mut self, timeout: Duration) -> Result<Buffers, Error>;
    fn consume_input(&mut self, amount: usize);
}

pub struct Buffers<'a> {
    pub input: &'a mut [u8],
    pub output: &'a mut [u8],
}

impl Buffers<'_> {
    pub(crate) fn empty() -> Buffers<'static> {
        Buffers {
            input: &mut [],
            output: &mut [],
        }
    }
}

#[derive(Debug)]
pub struct ChainedConnector {
    chain: Vec<Box<dyn Connector>>,
}

impl ChainedConnector {
    fn new(chain: impl IntoIterator<Item = Box<dyn Connector>>) -> Self {
        Self {
            chain: chain.into_iter().collect(),
        }
    }
}

impl Connector for ChainedConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        let mut conn = chained;

        for connector in &self.chain {
            conn = connector.connect(details, conn)?;
        }

        Ok(conn)
    }
}

#[derive(Debug)]
pub struct DefaultConnector {
    chain: ChainedConnector,
}

impl DefaultConnector {
    pub fn new() -> Self {
        let chain = ChainedConnector::new([
            TcpConnector.boxed(),
            #[cfg(feature = "rustls")]
            RustlsConnector::default().boxed(),
        ]);

        DefaultConnector { chain }
    }
}

impl Connector for DefaultConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        self.chain.connect(details, chained)
    }
}
