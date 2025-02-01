use std::fmt;
use std::marker::PhantomData;

use super::{Connector, Transport};

/// Two chained connectors called one after another.
///
/// Created by calling [`Connector::chain`] on the first connector.
pub struct ChainedConnector<In, First, Second>(First, Second, PhantomData<In>);

impl<In, First, Second> Connector<In> for ChainedConnector<In, First, Second>
where
    In: Transport,
    First: Connector<In>,
    Second: Connector<First::Out>,
{
    type Out = Second::Out;

    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let f_out = self.0.connect(details, chained)?;
        self.1.connect(details, f_out)
    }
}

impl<In, First, Second> ChainedConnector<In, First, Second> {
    pub(crate) fn new(first: First, second: Second) -> Self {
        ChainedConnector(first, second, PhantomData)
    }
}

impl<In, First, Second> fmt::Debug for ChainedConnector<In, First, Second>
where
    In: Transport,
    First: Connector<In>,
    Second: Connector<First::Out>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ChainedConnector")
            .field(&self.0)
            .field(&self.1)
            .finish()
    }
}

impl<In, First, Second> Clone for ChainedConnector<In, First, Second>
where
    In: Transport,
    First: Connector<In> + Clone,
    Second: Connector<First::Out> + Clone,
{
    fn clone(&self) -> Self {
        ChainedConnector(self.0.clone(), self.1.clone(), PhantomData)
    }
}

/// A selection between two transports.
#[derive(Debug)]
pub enum Either<A, B> {
    /// The first transport.
    A(A),
    /// The second transport.
    B(B),
}

impl<A: Transport, B: Transport> Transport for Either<A, B> {
    fn buffers(&mut self) -> &mut dyn super::Buffers {
        match self {
            Either::A(a) => a.buffers(),
            Either::B(b) => b.buffers(),
        }
    }

    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: super::NextTimeout,
    ) -> Result<(), crate::Error> {
        match self {
            Either::A(a) => a.transmit_output(amount, timeout),
            Either::B(b) => b.transmit_output(amount, timeout),
        }
    }

    fn await_input(&mut self, timeout: super::NextTimeout) -> Result<bool, crate::Error> {
        match self {
            Either::A(a) => a.await_input(timeout),
            Either::B(b) => b.await_input(timeout),
        }
    }

    fn is_open(&mut self) -> bool {
        match self {
            Either::A(a) => a.is_open(),
            Either::B(b) => b.is_open(),
        }
    }

    fn is_tls(&self) -> bool {
        match self {
            Either::A(a) => a.is_tls(),
            Either::B(b) => b.is_tls(),
        }
    }
}

// Connector is implemented for () to start a chain of connectors.
//
// The `Out` transport is supposedly `()`, but this is never instantiated.
impl Connector<()> for () {
    type Out = ();

    fn connect(
        &self,
        _: &super::ConnectionDetails,
        _: Option<()>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        Ok(None)
    }
}

// () is a valid Transport for type reasons.
//
// It should never be instantiated as an actual transport.
impl Transport for () {
    fn buffers(&mut self) -> &mut dyn super::Buffers {
        panic!("Unit transport is not valid")
    }

    fn transmit_output(&mut self, _: usize, _: super::NextTimeout) -> Result<(), crate::Error> {
        panic!("Unit transport is not valid")
    }

    fn await_input(&mut self, _: super::NextTimeout) -> Result<bool, crate::Error> {
        panic!("Unit transport is not valid")
    }

    fn is_open(&mut self) -> bool {
        panic!("Unit transport is not valid")
    }
}
