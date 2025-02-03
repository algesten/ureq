use super::{Connector, Transport};

/// Chain of up to 8 connectors
///
/// Can be created manually from a tuple of connectors through `ChainedConnector::new`
#[derive(Debug, Clone)]
pub struct ChainedConnector<Connectors>(Connectors);

impl<Connectors> ChainedConnector<Connectors> {
    /// Create a new chained connector that chains a tuple of connectors
    ///
    /// ```rust
    /// # use ureq::unversioned::transport::{ChainedConnector, TcpConnector, ConnectProxyConnector};
    /// let connector: ChainedConnector<(TcpConnector, ConnectProxyConnector)> =
    ///     ChainedConnector::new((
    ///         TcpConnector::default(),
    ///         ConnectProxyConnector::default()
    ///     ));
    /// ```
    pub fn new(connectors: Connectors) -> Self {
        Self(connectors)
    }
}

// Macro to generate the implementations of ChainedConnectors for various tuple length
// macro_rules! impl_chained_connectors {
//     (($first_ty:ident, $first_name: ident) ; $(($ty:ident, $name:ident, $prev_ty:ident)),* ; ($final_ty:ident, $final_name: ident, $pre_final_ty:ident)) => {
//         impl<In, $first_ty, $($ty,)* $final_ty> Connector<In> for ChainedConnector<($first_ty, $($ty,)* $final_ty)>
//         where
//             In: Transport,
//             $first_ty: Connector<In>,
//             $($ty: Connector<$prev_ty::Out>,)*
//             $final_ty: Connector<$pre_final_ty::Out>,
//         {
//             type Out = $final_ty::Out;
//             fn connect(
//                 &self,
//                 details: &super::ConnectionDetails,
//                 chained: Option<In>,
//             ) -> Result<Option<Self::Out>, crate::Error> {
//                 let ChainedConnector((
//                     ref $first_name,
//                     $(ref $name,)*
//                     ref $final_name,
//                 )) = self;

//                 let out = $first_name.connect(details, chained)?;
//                 $(
//                     let out = $name.connect(details, out)?;
//                 )*
//                 $final_name.connect(details,out)
//             }
//         }

//     };
// }

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;
//     (B, b, A),
//     (C, c, B),
//     (D, d, C),
//     (E, e, D),
//     (F, f, E),
//     (G, g, F),
//     (H, h, G),
//     (I, i, H);
//     (J, j, I)
// );
impl<In, A, B, C, D, E, F, G, H, I, J> Connector<In>
    for ChainedConnector<(A, B, C, D, E, F, G, H, I, J)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
    C: Connector<B::Out>,
    D: Connector<C::Out>,
    E: Connector<D::Out>,
    F: Connector<E::Out>,
    G: Connector<F::Out>,
    H: Connector<G::Out>,
    I: Connector<H::Out>,
    J: Connector<I::Out>,
{
    type Out = J::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((
            ref a,
            ref b,
            ref c,
            ref d,
            ref e,
            ref f,
            ref g,
            ref h,
            ref i,
            ref j,
        )) = self;
        let out = a.connect(details, chained)?;
        let out = b.connect(details, out)?;
        let out = c.connect(details, out)?;
        let out = d.connect(details, out)?;
        let out = e.connect(details, out)?;
        let out = f.connect(details, out)?;
        let out = g.connect(details, out)?;
        let out = h.connect(details, out)?;
        let out = i.connect(details, out)?;
        j.connect(details, out)
    }
}

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;
//     (B, b, A),
//     (C, c, B),
//     (D, d, C),
//     (E, e, D),
//     (F, f, E),
//     (G, g, F),
//     (H, h, G);
//     (I, i, H)
// );
impl<In, A, B, C, D, E, F, G, H, I> Connector<In> for ChainedConnector<(A, B, C, D, E, F, G, H, I)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
    C: Connector<B::Out>,
    D: Connector<C::Out>,
    E: Connector<D::Out>,
    F: Connector<E::Out>,
    G: Connector<F::Out>,
    H: Connector<G::Out>,
    I: Connector<H::Out>,
{
    type Out = I::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((ref a, ref b, ref c, ref d, ref e, ref f, ref g, ref h, ref i)) =
            self;
        let out = a.connect(details, chained)?;
        let out = b.connect(details, out)?;
        let out = c.connect(details, out)?;
        let out = d.connect(details, out)?;
        let out = e.connect(details, out)?;
        let out = f.connect(details, out)?;
        let out = g.connect(details, out)?;
        let out = h.connect(details, out)?;
        i.connect(details, out)
    }
}

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;
//     (B, b, A),
//     (C, c, B),
//     (D, d, C),
//     (E, e, D),
//     (F, f, E),
//     (G, g, F);
//     (H, h, G)
// );
impl<In, A, B, C, D, E, F, G, H> Connector<In> for ChainedConnector<(A, B, C, D, E, F, G, H)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
    C: Connector<B::Out>,
    D: Connector<C::Out>,
    E: Connector<D::Out>,
    F: Connector<E::Out>,
    G: Connector<F::Out>,
    H: Connector<G::Out>,
{
    type Out = H::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((ref a, ref b, ref c, ref d, ref e, ref f, ref g, ref h)) = self;
        let out = a.connect(details, chained)?;
        let out = b.connect(details, out)?;
        let out = c.connect(details, out)?;
        let out = d.connect(details, out)?;
        let out = e.connect(details, out)?;
        let out = f.connect(details, out)?;
        let out = g.connect(details, out)?;
        h.connect(details, out)
    }
}

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;
//     (B, b, A),
//     (C, c, B),
//     (D, d, C),
//     (E, e, D),
//     (F, f, E);
//     (G, g, F)
// );
impl<In, A, B, C, D, E, F, G> Connector<In> for ChainedConnector<(A, B, C, D, E, F, G)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
    C: Connector<B::Out>,
    D: Connector<C::Out>,
    E: Connector<D::Out>,
    F: Connector<E::Out>,
    G: Connector<F::Out>,
{
    type Out = G::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((ref a, ref b, ref c, ref d, ref e, ref f, ref g)) = self;
        let out = a.connect(details, chained)?;
        let out = b.connect(details, out)?;
        let out = c.connect(details, out)?;
        let out = d.connect(details, out)?;
        let out = e.connect(details, out)?;
        let out = f.connect(details, out)?;
        g.connect(details, out)
    }
}

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;
//     (B, b, A),
//     (C, c, B),
//     (D, d, C),
//     (E, e, D);
//     (F, f, E)
// );
impl<In, A, B, C, D, E, F> Connector<In> for ChainedConnector<(A, B, C, D, E, F)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
    C: Connector<B::Out>,
    D: Connector<C::Out>,
    E: Connector<D::Out>,
    F: Connector<E::Out>,
{
    type Out = F::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((ref a, ref b, ref c, ref d, ref e, ref f)) = self;
        let out = a.connect(details, chained)?;
        let out = b.connect(details, out)?;
        let out = c.connect(details, out)?;
        let out = d.connect(details, out)?;
        let out = e.connect(details, out)?;
        f.connect(details, out)
    }
}

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;
//     (B, b, A),
//     (C, c, B),
//     (D, d, C);
//     (E, e, D)
// );
impl<In, A, B, C, D, E> Connector<In> for ChainedConnector<(A, B, C, D, E)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
    C: Connector<B::Out>,
    D: Connector<C::Out>,
    E: Connector<D::Out>,
{
    type Out = E::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((ref a, ref b, ref c, ref d, ref e)) = self;
        let out = a.connect(details, chained)?;
        let out = b.connect(details, out)?;
        let out = c.connect(details, out)?;
        let out = d.connect(details, out)?;
        e.connect(details, out)
    }
}

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;
//     (B, b, A),
//     (C, c, B);
//     (D, d, C)
// );
impl<In, A, B, C, D> Connector<In> for ChainedConnector<(A, B, C, D)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
    C: Connector<B::Out>,
    D: Connector<C::Out>,
{
    type Out = D::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((ref a, ref b, ref c, ref d)) = self;
        let out = a.connect(details, chained)?;
        let out = b.connect(details, out)?;
        let out = c.connect(details, out)?;
        d.connect(details, out)
    }
}

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;
//     (B, b, A);
//     (C, c, B)
// );
impl<In, A, B, C> Connector<In> for ChainedConnector<(A, B, C)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
    C: Connector<B::Out>,
{
    type Out = C::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((ref a, ref b, ref c)) = self;
        let out = a.connect(details, chained)?;
        let out = b.connect(details, out)?;
        c.connect(details, out)
    }
}

// Expansion of:
// impl_chained_connectors!(
//     (A, a) ;;
//     (B, b, A)
// );
impl<In, A, B> Connector<In> for ChainedConnector<(A, B)>
where
    In: Transport,
    A: Connector<In>,
    B: Connector<A::Out>,
{
    type Out = B::Out;
    fn connect(
        &self,
        details: &super::ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, crate::Error> {
        let ChainedConnector((ref a, ref b)) = self;
        let out = a.connect(details, chained)?;
        b.connect(details, out)
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
