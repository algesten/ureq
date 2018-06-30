use agent::Unit;
use error::Error;
use std::io::{Cursor, Read, Result as IoResult, Write};
use std::net::SocketAddr;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;

#[cfg(feature = "tls")]
use native_tls::TlsStream;

pub enum Stream {
    Http(TcpStream),
    #[cfg(feature = "tls")]
    Https(TlsStream<TcpStream>),
    Cursor(Cursor<Vec<u8>>),
    #[cfg(test)]
    Test(Box<Read + Send>, Vec<u8>),
}

impl ::std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::result::Result<(), ::std::fmt::Error> {
        write!(
            f,
            "Stream[{}]",
            match self {
                Stream::Http(_) => "http",
                Stream::Https(_) => "https",
                Stream::Cursor(_) => "cursor",
                #[cfg(test)]
                Stream::Test(_, _) => "test",
            }
        )
    }
}

impl Stream {
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
            Stream::Https(stream) => stream.read(buf),
            Stream::Cursor(read) => read.read(buf),
            #[cfg(test)]
            Stream::Test(reader, _) => reader.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        match self {
            Stream::Http(sock) => sock.write(buf),
            Stream::Https(stream) => stream.write(buf),
            Stream::Cursor(_) => panic!("Write to read only stream"),
            #[cfg(test)]
            Stream::Test(_, writer) => writer.write(buf),
        }
    }
    fn flush(&mut self) -> IoResult<()> {
        match self {
            Stream::Http(sock) => sock.flush(),
            Stream::Https(stream) => stream.flush(),
            Stream::Cursor(_) => panic!("Flush read only stream"),
            #[cfg(test)]
            Stream::Test(_, writer) => writer.flush(),
        }
    }
}

pub fn connect_http(unit: &Unit) -> Result<Stream, Error> {
    //
    let hostname = unit.url.host_str().unwrap();
    let port = unit.url.port().unwrap_or(80);

    connect_host(unit, hostname, port).map(|tcp| Stream::Http(tcp))
}

#[cfg(feature = "tls")]
pub fn connect_https(unit: &Unit) -> Result<Stream, Error> {
    use native_tls::TlsConnector;

    let hostname = unit.url.host_str().unwrap();
    let port = unit.url.port().unwrap_or(443);

    let socket = connect_host(unit, hostname, port)?;
    let connector = TlsConnector::builder().build()?;
    let stream = connector.connect(hostname, socket)?;

    Ok(Stream::Https(stream))
}

pub fn connect_host(unit: &Unit, hostname: &str, port: u16) -> Result<TcpStream, Error> {
    //
    let ips: Vec<SocketAddr> = format!("{}:{}", hostname, port)
        .to_socket_addrs()
        .map_err(|e| Error::DnsFailed(format!("{}", e)))?
        .collect();

    if ips.len() == 0 {
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
    }.map_err(|err| Error::ConnectionFailed(format!("{}", err)))?;

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
pub fn connect_test(unit: &Unit) -> Result<Stream, Error> {
    use test;
    test::resolve_handler(unit)
}

#[cfg(not(test))]
pub fn connect_test(unit: &Unit) -> Result<Stream, Error> {
    Err(Error::UnknownScheme(unit.url.scheme().to_string()))
}

#[cfg(not(feature = "tls"))]
pub fn connect_https(unit: &Unit) -> Result<Stream, Error> {
    Err(Error::UnknownScheme(unit.url.scheme().to_string()))
}
