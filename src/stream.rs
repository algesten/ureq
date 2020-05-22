use std::io::{Cursor, ErrorKind, Read, Result as IoResult, Write};
use std::net::SocketAddr;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::time::Duration;

#[cfg(feature = "tls")]
use rustls::ClientSession;
#[cfg(feature = "tls")]
use rustls::StreamOwned;
#[cfg(feature = "socks-proxy")]
use socks::{TargetAddr, ToTargetAddr};

use crate::proxy::Proto;
use crate::proxy::Proxy;

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

#[allow(deprecated)]
fn is_close_notify(e: &std::io::Error) -> bool {
    if e.kind() != ErrorKind::ConnectionAborted {
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

#[cfg(all(feature = "tls", feature = "native-certs"))]
fn configure_certs(config: &mut rustls::ClientConfig) {
    config.root_store =
        rustls_native_certs::load_native_certs().expect("Could not load patform certs");
}

#[cfg(all(feature = "tls", not(feature = "native-certs")))]
fn configure_certs(config: &mut rustls::ClientConfig) {
    config
        .root_store
        .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
}

#[cfg(feature = "tls")]
pub(crate) fn connect_https(unit: &Unit) -> Result<Stream, Error> {
    use lazy_static::lazy_static;
    use std::sync::Arc;

    lazy_static! {
        static ref TLS_CONF: Arc<rustls::ClientConfig> = {
            let mut config = rustls::ClientConfig::new();
            configure_certs(&mut config);
            Arc::new(config)
        };
    }

    let hostname = unit.url.host_str().unwrap();
    let port = unit.url.port().unwrap_or(443);

    let sni = webpki::DNSNameRef::try_from_ascii_str(hostname)
        .map_err(|err| Error::DnsFailed(err.to_string()))?;
    let tls_conf: &Arc<rustls::ClientConfig> =
        unit.tls_config.as_ref().map(|c| &c.0).unwrap_or(&*TLS_CONF);
    let sess = rustls::ClientSession::new(&tls_conf, sni);

    let sock = connect_host(unit, hostname, port)?;

    let stream = rustls::StreamOwned::new(sess, sock);

    Ok(Stream::Https(stream))
}

pub(crate) fn connect_host(unit: &Unit, hostname: &str, port: u16) -> Result<TcpStream, Error> {
    //
    let sock_addrs: Vec<SocketAddr> = match unit.proxy {
        Some(ref proxy) => format!("{}:{}", proxy.server, proxy.port),
        None => format!("{}:{}", hostname, port),
    }
    .to_socket_addrs()
    .map_err(|e| Error::DnsFailed(format!("{}", e)))?
    .collect();

    let proto = if let Some(ref proxy) = unit.proxy {
        Some(proxy.proto)
    } else {
        None
    };

    let mut any_err = None;
    let mut any_stream = None;

    // Find the first sock_addr that accepts a connection
    for sock_addr in sock_addrs {
        // connect with a configured timeout.
        let stream = if Some(Proto::SOCKS5) == proto {
            connect_socks5(
                unit.proxy.to_owned().unwrap(),
                unit.timeout_connect,
                sock_addr,
                hostname,
                port,
            )
        } else {
            match unit.timeout_connect {
                0 => TcpStream::connect(&sock_addr),
                _ => TcpStream::connect_timeout(
                    &sock_addr,
                    Duration::from_millis(unit.timeout_connect as u64),
                ),
            }
        };

        if let Ok(stream) = stream {
            any_stream = Some(stream);
            break;
        } else if let Err(err) = stream {
            any_err = Some(err);
        }
    }

    let mut stream = if let Some(stream) = any_stream {
        stream
    } else {
        let err = if let Some(err) = any_err {
            Error::ConnectionFailed(format!("{}", err))
        } else {
            Error::DnsFailed(format!("No ip address for {}", hostname))
        };
        return Err(err);
    };

    // rust's absurd api returns Err if we set 0.
    // Setting it to None will disable the native system timeout
    if unit.timeout_read > 0 {
        stream
            .set_read_timeout(Some(Duration::from_millis(unit.timeout_read as u64)))
            .ok();
    } else {
        stream.set_read_timeout(None).ok();
    }

    if unit.timeout_write > 0 {
        stream
            .set_write_timeout(Some(Duration::from_millis(unit.timeout_write as u64)))
            .ok();
    } else {
        stream.set_write_timeout(None).ok();
    }

    if proto == Some(Proto::HTTPConnect) {
        if let Some(ref proxy) = unit.proxy {
            write!(stream, "{}", proxy.connect(hostname, port)).unwrap();
            stream.flush()?;

            let mut proxy_response = Vec::new();

            loop {
                let mut buf = vec![0; 256];
                let total = stream.read(&mut buf)?;
                proxy_response.append(&mut buf);
                if total < 256 {
                    break;
                }
            }

            Proxy::verify_response(&proxy_response)?;
        }
    }

    Ok(stream)
}

#[cfg(feature = "socks-proxy")]
fn socks5_local_nslookup(hostname: &str, port: u16) -> Result<TargetAddr, std::io::Error> {
    let addrs: Vec<SocketAddr> = format!("{}:{}", hostname, port)
        .to_socket_addrs()
        .map_err(|e| std::io::Error::new(ErrorKind::NotFound, format!("DNS failure: {}.", e)))?
        .collect();

    if addrs.is_empty() {
        return Err(std::io::Error::new(
            ErrorKind::NotFound,
            "DNS failure: no socket addrs found.",
        ));
    }

    match addrs[0].to_target_addr() {
        Ok(addr) => Ok(addr),
        Err(err) => {
            return Err(std::io::Error::new(
                ErrorKind::NotFound,
                format!("DNS failure: {}.", err),
            ))
        }
    }
}

#[cfg(feature = "socks-proxy")]
fn connect_socks5(
    proxy: Proxy,
    timeout_connect: u64,
    proxy_addr: SocketAddr,
    host: &str,
    port: u16,
) -> Result<TcpStream, std::io::Error> {
    use socks::TargetAddr::Domain;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::str::FromStr;

    let host_addr = if Ipv4Addr::from_str(host).is_ok() || Ipv6Addr::from_str(host).is_ok() {
        match socks5_local_nslookup(host, port) {
            Ok(addr) => addr,
            Err(err) => return Err(err),
        }
    } else {
        Domain(String::from(host), port)
    };

    // Since Socks5Stream doesn't support set_read_timeout, a suboptimal one is implemented via
    // thread::spawn.
    // # Happy Path
    // 1) thread spawns 2) get_socks5_stream returns ok 3) tx sends result ok
    // 4) slave_signal signals done and cvar notifies master_signal 5) cvar.wait_timeout receives the done signal
    // 6) rx receives the socks5 stream and the function exists
    // # Sad path
    // 1) get_socks5_stream hangs 2)slave_signal does not send done notification 3) cvar.wait_timeout times out
    // 3) an exception is thrown.
    // # Defects
    // 1) In the event of a timeout, a thread may be left running in the background.
    // TODO: explore supporting timeouts upstream in Socks5Proxy.
    #[allow(clippy::mutex_atomic)]
    let stream = if timeout_connect > 0 {
        use std::sync::mpsc::channel;
        use std::sync::{Arc, Condvar, Mutex};
        use std::thread;
        let master_signal = Arc::new((Mutex::new(false), Condvar::new()));
        let slave_signal = master_signal.clone();
        let (tx, rx) = channel();
        thread::spawn(move || {
            let (lock, cvar) = &*slave_signal;
            if tx // try to get a socks5 stream and send it to the parent thread's rx
                .send(get_socks5_stream(&proxy, &proxy_addr, host_addr))
                .is_ok()
            {
                // if sending the stream has succeeded we need to notify the parent thread
                let mut done = lock.lock().unwrap();
                // set the done signal to true
                *done = true;
                // notify the parent thread
                cvar.notify_one();
            }
        });

        let (lock, cvar) = &*master_signal;
        let done = lock.lock().unwrap();

        let done_result = cvar
            .wait_timeout(done, Duration::from_millis(timeout_connect))
            .unwrap();
        let done = done_result.0;
        if *done {
            rx.recv().unwrap()?
        } else {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                format!(
                    "SOCKS5 proxy: {}:{} timed out connecting after {}ms.",
                    host, port, timeout_connect
                ),
            ));
        }
    } else {
        get_socks5_stream(&proxy, &proxy_addr, host_addr)?
    };

    Ok(stream)
}

#[cfg(feature = "socks-proxy")]
fn get_socks5_stream(
    proxy: &Proxy,
    proxy_addr: &SocketAddr,
    host_addr: TargetAddr,
) -> Result<TcpStream, std::io::Error> {
    use socks::Socks5Stream;
    if proxy.use_authorization() {
        let stream = Socks5Stream::connect_with_password(
            proxy_addr,
            host_addr,
            &proxy.user.as_ref().unwrap(),
            &proxy.password.as_ref().unwrap(),
        )?
        .into_inner();
        Ok(stream)
    } else {
        match Socks5Stream::connect(proxy_addr, host_addr) {
            Ok(socks_stream) => Ok(socks_stream.into_inner()),
            Err(err) => Err(err),
        }
    }
}

#[cfg(not(feature = "socks-proxy"))]
fn connect_socks5(
    _proxy: Proxy,
    _timeout_connect: u64,
    _proxy_addr: SocketAddr,
    _hostname: &str,
    _port: u16,
) -> Result<TcpStream, std::io::Error> {
    Err(std::io::Error::new(
        ErrorKind::Other,
        "SOCKS5 feature disabled.",
    ))
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
