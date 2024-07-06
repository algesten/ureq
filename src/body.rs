// TODO(martin): Is a better name SendBody to  mirror RecvBody? Or even invert
// BodySend, BodyRecv to get alphabetical sorting.

use std::fs::File;
use std::io::Read;

pub struct Body<'a> {
    inner: BodyInner<'a>,
    ended: bool,
}

impl<'a> Body<'a> {
    pub fn empty() -> Body<'static> {
        Body {
            inner: BodyInner::ByteSlice(&[]),
            ended: false,
        }
    }
}

pub trait AsBody {
    fn as_body(&self) -> Body;
}

pub(crate) enum BodyInner<'a> {
    ByteSlice(&'a [u8]),
    Reader(&'a mut dyn Read),
}

macro_rules! impl_into_body {
    ($t:ty) => {
        impl AsBody for $t {
            fn as_body(&self) -> Body {
                BodyInner::ByteSlice(self.as_ref()).into()
            }
        }
    };
}

impl AsBody for &mut File {
    fn as_body(&self) -> Body {
        unreachable!()
    }
}

impl_into_body!(&[u8]);
impl_into_body!(&str);
impl_into_body!(String);
impl_into_body!(Vec<u8>);
impl_into_body!(&String);
impl_into_body!(&Vec<u8>);

pub struct RecvBody;

impl<'a> From<BodyInner<'a>> for Body<'a> {
    fn from(inner: BodyInner<'a>) -> Self {
        Body {
            inner,
            ended: false,
        }
    }
}
