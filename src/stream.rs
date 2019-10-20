use std::io::{Cursor, Read, Result as IoResult, Write};
use std::net::SocketAddr;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;

#[cfg(feature = "tls")]
use rustls::ClientSession;
#[cfg(feature = "tls")]
use rustls::StreamOwned;

use crate::error::Error;
use crate::unit::Unit;

#[allow(clippy::large_enum_variant)]
pub enum Stream {
    Http(TcpStream),
    #[cfg(feature = "tls")]
    Https(rustls::StreamOwned<rustls::ClientSession, TcpStream>),
    Cursor(Cursor<Vec<u8>>),
    #[cfg(test)]
    Test(Box<dyn Read + Send>, Vec<u8>),
}

impl ::std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(
            f,
            "Stream[{}]",
            match self {
                Stream::Http(_) => "http",
                #[cfg(feature = "tls")]
                Stream::Https(_) => "https",
                Stream::Cursor(_) => "cursor",
                #[cfg(test)]
                Stream::Test(_, _) => "test",
            }
        )
    }
}

impl Stream {
    pub fn is_poolable(&self) -> bool {
        match self {
            Stream::Http(_) => true,
            #[cfg(feature = "tls")]
            Stream::Https(_) => true,
            _ => false,
        }
    }

    #[cfg(test)]
    pub fn to_write_vec(&self) -> Vec<u8> {
        match self {
            Stream::Test(_, writer) => writer.clone(),
            _ => panic!("to_write_vec on non Test stream"),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match self {
            Stream::Http(sock) => sock.read(buf),
            #[cfg(feature = "tls")]
            Stream::Https(stream) => read_https(stream, buf),
            Stream::Cursor(read) => read.read(buf),
            #[cfg(test)]
            Stream::Test(reader, _) => reader.read(buf),
        }
    }
}

#[cfg(feature = "tls")]
fn read_https(
    stream: &mut StreamOwned<ClientSession, TcpStream>,
    buf: &mut [u8],
) -> IoResult<usize> {
    match stream.read(buf) {
        Ok(size) => Ok(size),
        Err(ref e) if is_close_notify(e) => Ok(0),
        Err(e) => Err(e),
    }
}

fn is_close_notify(e: &std::io::Error) -> bool {
    if e.kind() != std::io::ErrorKind::ConnectionAborted {
        return false;
    }

    if let Some(msg) = e.get_ref() {
        // :(

        return msg.description().contains("CloseNotify");
    }

    false
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        match self {
            Stream::Http(sock) => sock.write(buf),
            #[cfg(feature = "tls")]
            Stream::Https(stream) => stream.write(buf),
            Stream::Cursor(_) => panic!("Write to read only stream"),
            #[cfg(test)]
            Stream::Test(_, writer) => writer.write(buf),
        }
    }
    fn flush(&mut self) -> IoResult<()> {
        match self {
            Stream::Http(sock) => sock.flush(),
            #[cfg(feature = "tls")]
            Stream::Https(stream) => stream.flush(),
            Stream::Cursor(_) => panic!("Flush read only stream"),
            #[cfg(test)]
            Stream::Test(_, writer) => writer.flush(),
        }
    }
}

pub(crate) fn connect_http(unit: &Unit) -> Result<Stream, Error> {
    //
    let hostname = unit.url.host_str().unwrap();
    let port = unit.url.port().unwrap_or(80);

    connect_host(unit, hostname, port).map(Stream::Http)
}

#[cfg(feature = "tls")]
pub(crate) fn connect_https(unit: &Unit) -> Result<Stream, Error> {
    use lazy_static::lazy_static;
    use std::sync::Arc;

    lazy_static! {
        static ref TLS_CONF: Arc<rustls::ClientConfig> = {
            let mut config = rustls::ClientConfig::new();
            config
                .root_store
                .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
            Arc::new(config)
        };
    }

    let hostname = unit.url.host_str().unwrap();
    let port = unit.url.port().unwrap_or(443);

    let sni = webpki::DNSNameRef::try_from_ascii_str(hostname).unwrap();
    let sess = rustls::ClientSession::new(&*TLS_CONF, sni);

    let sock = connect_host(unit, hostname, port)?;

    let stream = rustls::StreamOwned::new(sess, sock);

    Ok(Stream::Https(stream))
}

pub(crate) fn connect_host(unit: &Unit, hostname: &str, port: u16) -> Result<TcpStream, Error> {
    //
    let ips: Vec<SocketAddr> = format!("{}:{}", hostname, port)
        .to_socket_addrs()
        .map_err(|e| Error::DnsFailed(format!("{}", e)))?
        .collect();

    if ips.is_empty() {
        return Err(Error::DnsFailed(format!("No ip address for {}", hostname)));
    }

    // pick first ip, or should we randomize?
    let sock_addr = ips[0];

    // connect with a configured timeout.
    let stream = match unit.timeout_connect {
        0 => TcpStream::connect(&sock_addr),
        _ => TcpStream::connect_timeout(
            &sock_addr,
            Duration::from_millis(unit.timeout_connect as u64),
        ),
    }
    .map_err(|err| Error::ConnectionFailed(format!("{}", err)))?;

    // rust's absurd api returns Err if we set 0.
    if unit.timeout_read > 0 {
        stream
            .set_read_timeout(Some(Duration::from_millis(unit.timeout_read as u64)))
            .ok();
    }
    if unit.timeout_write > 0 {
        stream
            .set_write_timeout(Some(Duration::from_millis(unit.timeout_write as u64)))
            .ok();
    }

    Ok(stream)
}

#[cfg(test)]
pub(crate) fn connect_test(unit: &Unit) -> Result<Stream, Error> {
    use crate::test;
    test::resolve_handler(unit)
}

#[cfg(not(test))]
pub(crate) fn connect_test(unit: &Unit) -> Result<Stream, Error> {
    Err(Error::UnknownScheme(unit.url.scheme().to_string()))
}

#[cfg(not(feature = "tls"))]
pub(crate) fn connect_https(unit: &Unit) -> Result<Stream, Error> {
    Err(Error::UnknownScheme(unit.url.scheme().to_string()))
}
