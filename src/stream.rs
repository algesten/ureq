use dns_lookup;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time::Duration;

#[cfg(feature = "tls")]
use native_tls::TlsConnector;
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

fn connect_http(request: &Request, url: &Url) -> Result<Stream, Error> {
    //
    let hostname = url.host_str().unwrap();
    let port = url.port().unwrap_or(80);

    connect_host(request, hostname, port).map(|tcp| Stream::Http(tcp))
}

#[cfg(feature = "tls")]
fn connect_https(request: &Request, url: &Url) -> Result<Stream, Error> {
    //
    let hostname = url.host_str().unwrap();
    let port = url.port().unwrap_or(443);

    let socket = connect_host(request, hostname, port)?;
    let connector = TlsConnector::builder()?.build()?;
    let stream = connector.connect(hostname, socket)?;

    Ok(Stream::Https(stream))
}

fn connect_host(request: &Request, hostname: &str, port: u16) -> Result<TcpStream, Error> {
    //
    let ips: Vec<IpAddr> =
        dns_lookup::lookup_host(hostname).map_err(|e| Error::DnsFailed(format!("{}", e)))?;

    if ips.len() == 0 {
        return Err(Error::DnsFailed(format!("No ip address for {}", hostname)));
    }

    // pick first ip, or should we randomize?
    let sock_addr = SocketAddr::new(ips[0], port);

    // connect with a configured timeout.
    let stream = match request.timeout {
        0 => TcpStream::connect(&sock_addr),
        _ => TcpStream::connect_timeout(&sock_addr, Duration::from_millis(request.timeout as u64)),
    }.map_err(|err| Error::ConnectionFailed(format!("{}", err)))?;

    // rust's absurd api returns Err if we set 0.
    if request.timeout_read > 0 {
        stream
            .set_read_timeout(Some(Duration::from_millis(request.timeout_read as u64)))
            .ok();
    }
    if request.timeout_write > 0 {
        stream
            .set_write_timeout(Some(Duration::from_millis(request.timeout_write as u64)))
            .ok();
    }

    Ok(stream)
}

#[cfg(test)]
fn connect_test(request: &Request, url: &Url) -> Result<Stream, Error> {
    use test;
    test::resolve_handler(request, url)
}

#[cfg(not(test))]
fn connect_test(_request: &Request, url: &Url) -> Result<Stream, Error> {
    Err(Error::UnknownScheme(url.scheme().to_string()))
}

#[cfg(not(feature = "tls"))]
fn connect_https(request: &Request, url: &Url) -> Result<Stream, Error> {
    Err(Error::UnknownScheme(url.scheme().to_string()))
}
