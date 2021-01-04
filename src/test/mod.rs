use crate::error::Error;
use crate::stream::Stream;
use crate::unit::Unit;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

mod agent_test;
mod body_read;
mod body_send;
mod query_string;
mod range;
mod redirect;
mod simple;
mod timeout;

type RequestHandler = dyn Fn(&Unit) -> Result<Stream, Error> + Send + 'static;

pub(crate) static TEST_HANDLERS: Lazy<Arc<Mutex<HashMap<String, Box<RequestHandler>>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

pub(crate) fn set_handler<H>(path: &str, handler: H)
where
    H: Fn(&Unit) -> Result<Stream, Error> + Send + 'static,
{
    let path = path.to_string();
    let handler = Box::new(handler);
    // See `resolve_handler` below for why poisoning isn't necessary.
    let mut handlers = match TEST_HANDLERS.lock() {
        Ok(h) => h,
        Err(poison) => poison.into_inner(),
    };
    handlers.insert(path, handler);
}

#[allow(clippy::write_with_newline)]
pub(crate) fn make_response(
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
    Ok(Stream::from_vec(buf))
}

pub(crate) fn resolve_handler(unit: &Unit) -> Result<Stream, Error> {
    let path = unit.url.path();
    // The only way this can panic is if
    // 1. `remove(path).unwrap()` panics, in which case the HANDLERS haven't been modified.
    // 2. `make_hash` for `handlers.insert` panics (in `set_handler`), in which case the HANDLERS haven't been modified.
    // In all cases, another test will fail as a result, so it's ok to continue other tests in parallel.
    let mut handlers = match TEST_HANDLERS.lock() {
        Ok(h) => h,
        Err(poison) => poison.into_inner(),
    };
    let handler = handlers.remove(path)
        .unwrap_or_else(|| panic!("call make_response(\"{}\") before fetching it in tests (or if you did make it, avoid fetching it more than once)", path));
    drop(handlers);
    handler(unit)
}
