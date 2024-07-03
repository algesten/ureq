// TODO(martin): Is a better name SendBody to  mirror RecvBody? Or even invert
// BodySend, BodyRecv to get alphabetical sorting.

pub trait Body {}

pub struct BodyOwned;

impl Body for BodyOwned {}
impl Body for &BodyOwned {}

impl BodyOwned {
    pub(crate) fn empty() -> BodyOwned {
        BodyOwned
    }
}

impl Body for &[u8] {}

pub struct RecvBody;
