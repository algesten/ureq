use crate::error::Error;
use crate::stream::Stream;
use crate::unit::Unit;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

mod agent_test;
mod auth;
mod body_read;
mod body_send;
mod query_string;
mod range;
mod redirect;
mod simple;
mod testserver;
mod timeout;

type RequestHandler = dyn Fn(&Unit) -> Result<Stream, Error> + Send + 'static;

lazy_static! {
    pub(crate) static ref TEST_HANDLERS: Arc<Mutex<HashMap<String, Box<RequestHandler>>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

pub(crate) fn set_handler<H>(path: &str, handler: H)
where
    H: Fn(&Unit) -> Result<Stream, Error> + Send + 'static,
{
    let mut handlers = TEST_HANDLERS.lock().unwrap();
    handlers.insert(path.to_string(), Box::new(handler));
}

#[allow(clippy::write_with_newline)]
pub fn make_response(
    status: u16,
    status_text: &str,
    headers: Vec<&str>,
    mut body: Vec<u8>,
) -> Result<Stream, Error> {
    let mut buf: Vec<u8> = vec![];
    write!(&mut buf, "HTTP/1.1 {} {}\r\n", status, status_text).ok();
    for hstr in headers.iter() {
        write!(&mut buf, "{}\r\n", hstr).ok();
    }
    write!(&mut buf, "\r\n").ok();
    buf.append(&mut body);
    let cursor = Cursor::new(buf);
    let write: Vec<u8> = vec![];
    Ok(Stream::Test(Box::new(cursor), write))
}

pub(crate) fn resolve_handler(unit: &Unit) -> Result<Stream, Error> {
    let mut handlers = TEST_HANDLERS.lock().unwrap();
    let path = unit.url.path();
    let handler = handlers.remove(path).unwrap();
    handler(unit)
}
