use alloc::{boxed::Box, vec::Vec};

use crate::Error;

use super::{ConnectionDetails, Connector, Transport};

/// Helper for a chain of connectors.
///
/// Each step of the chain, can decide whether to:
///
/// * _Keep_ previous [`Transport`]
/// * _Wrap_ previous [`Transport`]
/// * _Ignore_ previous [`Transport`] in favor of some other connection.
///
/// For each new connection, the chain will be called one by one and the previously chained
/// transport will be provided to the next as an argument in [`Connector::connect()`].
///
/// The chain is always looped fully. There is no early return.
#[derive(Debug)]
pub struct ChainedConnector {
    chain: Vec<Box<dyn Connector>>,
}

impl ChainedConnector {
    /// Creates a new chain of connectors.
    ///
    /// For each connection, the chain will be called one by one and the previously chained
    /// transport will be provided to the next as an argument in [`Connector::connect()`].
    ///
    /// The chain is always looped fully. There is no early return.
    pub fn new(chain: impl IntoIterator<Item = Box<dyn Connector>>) -> Self {
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
