// TODO(martin): Is a better name SendBody to  mirror RecvBody? Or even invert
// BodySend, BodyRecv to get alphabetical sorting.

use std::io::Read;

pub struct Body<'a> {
    inner: BodyInner<'a>,
}

impl<'a> Body<'a> {
    pub fn empty() -> Body<'static> {
        Body {
            inner: BodyInner::ByteSlice(&[]),
        }
    }
}

pub trait IntoBody {
    fn as_body(&self) -> Body;
}

pub(crate) enum BodyInner<'a> {
    ByteSlice(&'a [u8]),
    Reader(&'a mut dyn Read),
}

impl IntoBody for &[u8] {
    fn as_body(&self) -> Body {
        BodyInner::ByteSlice(self).into()
    }
}

pub struct RecvBody;

impl<'a> From<BodyInner<'a>> for Body<'a> {
    fn from(inner: BodyInner<'a>) -> Self {
        Body { inner }
    }
}
