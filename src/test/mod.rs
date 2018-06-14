use agent::Request;
use agent::Stream;
use error::Error;
use header::Header;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use url::Url;
use std::io::Cursor;

mod agent_test;
mod auth;
mod body_read;
mod simple;

type RequestHandler = Fn(&Request, &Url) -> Result<Stream, Error> + Send + 'static;

lazy_static! {
    pub static ref TEST_HANDLERS: Arc<Mutex<HashMap<String, Box<RequestHandler>>>> =
        { Arc::new(Mutex::new(HashMap::new())) };
}

pub fn set_handler<H>(path: &str, handler: H)
where
    H: Fn(&Request, &Url) -> Result<Stream, Error> + Send + 'static,
{
    let mut handlers = TEST_HANDLERS.lock().unwrap();
    handlers.insert(path.to_string(), Box::new(handler));
}

pub fn make_response(
    status: u16,
    status_text: &str,
    headers: Vec<&str>,
    mut body: Vec<u8>,
) -> Result<Stream, Error> {
    let mut buf: Vec<u8> = vec![];
    write!(&mut buf, "HTTP/1.1 {} {}\r\n", status, status_text).ok();
    for hstr in headers.iter() {
        let header = hstr.parse::<Header>().unwrap();
        write!(&mut buf, "{}: {}\r\n", header.name(), header.value()).ok();
    }
    write!(&mut buf, "\r\n").ok();
    buf.append(&mut body);
    let cursor = Cursor::new(buf);
    let write: Vec<u8> = vec![];
    Ok(Stream::Test(Box::new(cursor), write))
}

pub fn resolve_handler(req: &Request, url: &Url) -> Result<Stream, Error> {
    let mut handlers = TEST_HANDLERS.lock().unwrap();
    let path = url.path();
    let handler = handlers.remove(path).unwrap();
    handler(req, url)
}
