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
