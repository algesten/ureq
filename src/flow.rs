use hoot::client::flow::state::*;
use hoot::client::flow::Flow;

pub(crate) enum FlowHolder<'a, B> {
    Prepare(Flow<'a, B, Prepare>),
    SendRequest(Flow<'a, B, SendRequest>),
    SendBody(Flow<'a, B, SendBody>),
    Await100(Flow<'a, B, Await100>),
    RecvResponse(Flow<'a, B, RecvResponse>),
    RecvBody(Flow<'a, B, RecvBody>),
    Redirect(Flow<'a, B, Redirect>),
    Cleanup(Flow<'a, B, Cleanup>),
}
