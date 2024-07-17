use crate::Error;

use super::{ConnectionDetails, Connector, Transport};

#[derive(Debug)]
pub struct ChainedConnector {
    chain: Vec<Box<dyn Connector>>,
}

impl ChainedConnector {
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
