use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::Duration;

use http::Uri;

use crate::proxy::Proxy;
use crate::resolver::Resolver;
use crate::tls::NativeTlsConnector;
use crate::{AgentConfig, Error};

#[cfg(feature = "rustls")]
use crate::tls::RustlsConnector;

use self::tcp::TcpConnector;

mod buf;
pub use buf::{Buffers, LazyBuffers, NoBuffers};

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
    fn buffers(&mut self) -> &mut dyn Buffers;
    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error>;
    fn await_input(&mut self, timeout: Duration) -> Result<(), Error>;
    fn consume_input(&mut self, amount: usize);
    fn is_tls(&self) -> bool {
        false
    }
}

// fn buffer_output(&mut self) -> &mut [u8];
// fn buffer_tmp_and_output(&mut self) -> (&mut [u8], &mut [u8]);
// fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error>;
// fn await_input(&mut self, timeout: Duration) -> Result<&[u8], Error>;
// fn consume_input(&mut self, amount: usize);

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
            #[cfg(feature = "native-tls")]
            NativeTlsConnector::default().boxed(),
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
