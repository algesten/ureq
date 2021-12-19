use log::debug;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time::Duration;
use std::time::Instant;
use std::{fmt, io::Cursor};

use chunked_transfer::Decoder as ChunkDecoder;

#[cfg(feature = "socks-proxy")]
use socks::{TargetAddr, ToTargetAddr};

use crate::proxy::Proxy;
use crate::{error::Error, proxy::Proto};

use crate::error::ErrorKind;
use crate::unit::Unit;

pub trait HttpsStream: Read + Write + Send + Sync + 'static {
    fn socket(&self) -> Option<&TcpStream>;
}

pub trait TlsConnector: Send + Sync {
    fn connect(
        &self,
        dns_name: &str,
        tcp_stream: TcpStream,
    ) -> Result<Box<dyn HttpsStream>, crate::error::Error>;
}

pub(crate) struct Stream {
    inner: BufReader<Box<dyn Inner + Send + Sync + 'static>>,
}

trait Inner: Read + Write {
    fn is_poolable(&self) -> bool;
    fn socket(&self) -> Option<&TcpStream>;
    fn as_write_vec(&self) -> &[u8] {
        panic!("as_write_vec on non Test stream");
    }
}

impl<T: HttpsStream + ?Sized> HttpsStream for Box<T> {
    fn socket(&self) -> Option<&TcpStream> {
        HttpsStream::socket(self.as_ref())
    }
}

impl<T: HttpsStream> Inner for T {
    fn is_poolable(&self) -> bool {
        true
    }

    fn socket(&self) -> Option<&TcpStream> {
        HttpsStream::socket(self)
    }
}

impl Inner for TcpStream {
    fn is_poolable(&self) -> bool {
        true
    }
    fn socket(&self) -> Option<&TcpStream> {
        Some(self)
    }
}

struct TestStream(Box<dyn Read + Send + Sync>, Vec<u8>);

impl Inner for TestStream {
    fn is_poolable(&self) -> bool {
        false
    }
    fn socket(&self) -> Option<&TcpStream> {
        None
    }
    fn as_write_vec(&self) -> &[u8] {
        &self.1
    }
}

impl Read for TestStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for TestStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.1.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// DeadlineStream wraps a stream such that read() will return an error
// after the provided deadline, and sets timeouts on the underlying
// TcpStream to ensure read() doesn't block beyond the deadline.
// When the From trait is used to turn a DeadlineStream back into a
// Stream (by PoolReturningRead), the timeouts are removed.
pub(crate) struct DeadlineStream {
    stream: Stream,
    deadline: Option<Instant>,
}

impl DeadlineStream {
    pub(crate) fn new(stream: Stream, deadline: Option<Instant>) -> Self {
        DeadlineStream { stream, deadline }
    }
}

impl From<DeadlineStream> for Stream {
    fn from(deadline_stream: DeadlineStream) -> Stream {
        deadline_stream.stream
    }
}

impl BufRead for DeadlineStream {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if let Some(deadline) = self.deadline {
            let timeout = time_until_deadline(deadline)?;
            if let Some(socket) = self.stream.socket() {
                socket.set_read_timeout(Some(timeout))?;
                socket.set_write_timeout(Some(timeout))?;
            }
        }
        self.stream.fill_buf().map_err(|e| {
            // On unix-y platforms set_read_timeout and set_write_timeout
            // causes ErrorKind::WouldBlock instead of ErrorKind::TimedOut.
            // Since the socket most definitely not set_nonblocking(true),
            // we can safely normalize WouldBlock to TimedOut
            if e.kind() == io::ErrorKind::WouldBlock {
                return io_err_timeout("timed out reading response".to_string());
            }
            e
        })
    }

    fn consume(&mut self, amt: usize) {
        self.stream.consume(amt)
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // All reads on a DeadlineStream use the BufRead impl. This ensures
        // that we have a chance to set the correct timeout before each recv
        // syscall.
        // Copied from the BufReader implementation of `read()`.
        let nread = {
            let mut rem = self.fill_buf()?;
            rem.read(buf)?
        };
        self.consume(nread);
        Ok(nread)
    }
}

// If the deadline is in the future, return the remaining time until
// then. Otherwise return a TimedOut error.
fn time_until_deadline(deadline: Instant) -> io::Result<Duration> {
    let now = Instant::now();
    match deadline.checked_duration_since(now) {
        None => Err(io_err_timeout("timed out reading response".to_string())),
        Some(duration) => Ok(duration),
    }
}

pub(crate) fn io_err_timeout(error: String) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, error)
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.inner.get_ref().socket() {
            Some(s) => write!(f, "{:?}", s),
            None => write!(f, "Stream(Test)"),
        }
    }
}

impl Stream {
    fn new(t: impl Inner + Send + Sync + 'static) -> Stream {
        Stream::logged_create(Stream {
            inner: BufReader::new(Box::new(t)),
        })
    }

    fn logged_create(stream: Stream) -> Stream {
        debug!("created stream: {:?}", stream);
        stream
    }

    pub(crate) fn from_vec(v: Vec<u8>) -> Stream {
        Stream::logged_create(Stream {
            inner: BufReader::new(Box::new(TestStream(Box::new(Cursor::new(v)), vec![]))),
        })
    }

    fn from_tcp_stream(t: TcpStream) -> Stream {
        Stream::logged_create(Stream {
            inner: BufReader::new(Box::new(t)),
        })
    }

    // Check if the server has closed a stream by performing a one-byte
    // non-blocking read. If this returns EOF, the server has closed the
    // connection: return true. If this returns a successful read, there are
    // some bytes on the connection even though there was no inflight request.
    // For plain HTTP streams, that might mean an HTTP 408 was pushed; it
    // could also mean a buggy server that sent more bytes than a response's
    // Content-Length. For HTTPS streams, that might mean a close_notify alert,
    // which is the proper way to shut down an idle stream.
    // Either way, bytes available on the stream before we've made a request
    // means the stream is not usable, so we should discard it.
    // If this returns WouldBlock (aka EAGAIN),
    // that means the connection is still open: return false. Otherwise
    // return an error.
    fn serverclosed_stream(stream: &std::net::TcpStream) -> io::Result<bool> {
        let mut buf = [0; 1];
        stream.set_nonblocking(true)?;

        let result = match stream.peek(&mut buf) {
            Ok(n) => {
                debug!(
                    "peek on reused connection returned {}, not WouldBlock; discarding",
                    n
                );
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(false),
            Err(e) => Err(e),
        };
        stream.set_nonblocking(false)?;

        result
    }
    // Return true if the server has closed this connection.
    pub(crate) fn server_closed(&self) -> io::Result<bool> {
        match self.socket() {
            Some(socket) => Stream::serverclosed_stream(socket),
            None => Ok(false),
        }
    }
    pub fn is_poolable(&self) -> bool {
        self.inner.get_ref().is_poolable()
    }

    pub(crate) fn reset(&mut self) -> io::Result<()> {
        // When we are turning this back into a regular, non-deadline Stream,
        // remove any timeouts we set.
        if let Some(socket) = self.socket() {
            socket.set_read_timeout(None)?;
            socket.set_write_timeout(None)?;
        }

        Ok(())
    }

    pub(crate) fn socket(&self) -> Option<&TcpStream> {
        self.inner.get_ref().socket()
    }

    pub(crate) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        if let Some(socket) = self.socket() {
            socket.set_read_timeout(timeout)
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    pub fn as_write_vec(&self) -> &[u8] {
        self.inner.get_ref().as_write_vec()
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl BufRead for Stream {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

impl<R: Read> From<ChunkDecoder<R>> for Stream
where
    R: Read,
    Stream: From<R>,
{
    fn from(chunk_decoder: ChunkDecoder<R>) -> Stream {
        chunk_decoder.into_inner().into()
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.get_mut().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.get_mut().flush()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        debug!("dropping stream: {:?}", self);
    }
}

pub(crate) fn connect_http(unit: &Unit, hostname: &str) -> Result<Stream, Error> {
    //
    let port = unit.url.port().unwrap_or(80);

    connect_host(unit, hostname, port).map(Stream::from_tcp_stream)
}

pub(crate) fn connect_https(unit: &Unit, hostname: &str) -> Result<Stream, Error> {
    let port = unit.url.port().unwrap_or(443);

    let sock = connect_host(unit, hostname, port)?;

    let tls_conf = &unit.agent.config.tls_config;
    let https_stream = tls_conf.connect(hostname, sock)?;
    Ok(Stream::new(https_stream))
}

pub(crate) fn connect_host(unit: &Unit, hostname: &str, port: u16) -> Result<TcpStream, Error> {
    let connect_deadline: Option<Instant> =
        if let Some(timeout_connect) = unit.agent.config.timeout_connect {
            Instant::now().checked_add(timeout_connect)
        } else {
            unit.deadline
        };
    let proxy: Option<Proxy> = unit.agent.config.proxy.clone();
    let netloc = match proxy {
        Some(ref proxy) => format!("{}:{}", proxy.server, proxy.port),
        None => format!("{}:{}", hostname, port),
    };

    // TODO: Find a way to apply deadline to DNS lookup.
    let sock_addrs = unit.resolver().resolve(&netloc).map_err(|e| {
        ErrorKind::Dns
            .msg(format!("resolve dns name '{}'", netloc))
            .src(e)
    })?;

    if sock_addrs.is_empty() {
        return Err(ErrorKind::Dns.msg(format!("No ip address for {}", hostname)));
    }

    let proto = proxy.as_ref().map(|proxy| proxy.proto);

    let mut any_err = None;
    let mut any_stream = None;
    // Find the first sock_addr that accepts a connection
    for sock_addr in sock_addrs {
        // ensure connect timeout or overall timeout aren't yet hit.
        let timeout = match connect_deadline {
            Some(deadline) => Some(time_until_deadline(deadline)?),
            None => None,
        };

        debug!("connecting to {} at {}", netloc, &sock_addr);

        // connect with a configured timeout.
        let stream = if None != proto && Some(Proto::HTTPConnect) != proto {
            connect_socks(
                unit,
                proxy.clone().unwrap(),
                connect_deadline,
                sock_addr,
                hostname,
                port,
                proto.unwrap(),
            )
        } else if let Some(timeout) = timeout {
            TcpStream::connect_timeout(&sock_addr, timeout)
        } else {
            TcpStream::connect(&sock_addr)
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
    } else if let Some(e) = any_err {
        return Err(ErrorKind::ConnectionFailed.msg("Connect error").src(e));
    } else {
        panic!("shouldn't happen: failed to connect to all IPs, but no error");
    };

    if let Some(deadline) = unit.deadline {
        stream.set_read_timeout(Some(time_until_deadline(deadline)?))?;
    } else {
        stream.set_read_timeout(unit.agent.config.timeout_read)?;
    }

    if let Some(deadline) = unit.deadline {
        stream.set_write_timeout(Some(time_until_deadline(deadline)?))?;
    } else {
        stream.set_write_timeout(unit.agent.config.timeout_write)?;
    }

    if proto == Some(Proto::HTTPConnect) {
        if let Some(ref proxy) = proxy {
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
fn socks_local_nslookup(
    unit: &Unit,
    hostname: &str,
    port: u16,
) -> Result<TargetAddr, std::io::Error> {
    let addrs: Vec<SocketAddr> = unit
        .resolver()
        .resolve(&format!("{}:{}", hostname, port))
        .map_err(|e| {
            std::io::Error::new(io::ErrorKind::NotFound, format!("DNS failure: {}.", e))
        })?;

    if addrs.is_empty() {
        return Err(std::io::Error::new(
            io::ErrorKind::NotFound,
            "DNS failure: no socket addrs found.",
        ));
    }

    match addrs[0].to_target_addr() {
        Ok(addr) => Ok(addr),
        Err(err) => {
            return Err(std::io::Error::new(
                io::ErrorKind::NotFound,
                format!("DNS failure: {}.", err),
            ))
        }
    }
}

#[cfg(feature = "socks-proxy")]
fn connect_socks(
    unit: &Unit,
    proxy: Proxy,
    deadline: Option<Instant>,
    proxy_addr: SocketAddr,
    host: &str,
    port: u16,
    proto: Proto,
) -> Result<TcpStream, std::io::Error> {
    use socks::TargetAddr::Domain;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::str::FromStr;

    let host_addr = if Ipv4Addr::from_str(host).is_ok()
        || Ipv6Addr::from_str(host).is_ok()
        || proto == Proto::SOCKS4
    {
        match socks_local_nslookup(unit, host, port) {
            Ok(addr) => addr,
            Err(err) => return Err(err),
        }
    } else {
        Domain(String::from(host), port)
    };

    // Since SocksXStream doesn't support set_read_timeout, a suboptimal one is implemented via
    // thread::spawn.
    // # Happy Path
    // 1) thread spawns 2) get_socksX_stream returns ok 3) tx sends result ok
    // 4) slave_signal signals done and cvar notifies master_signal 5) cvar.wait_timeout receives the done signal
    // 6) rx receives the socks5 stream and the function exists
    // # Sad path
    // 1) get_socksX_stream hangs 2)slave_signal does not send done notification 3) cvar.wait_timeout times out
    // 3) an exception is thrown.
    // # Defects
    // 1) In the event of a timeout, a thread may be left running in the background.
    // TODO: explore supporting timeouts upstream in Socks5Proxy.
    #[allow(clippy::mutex_atomic)]
    let stream = if let Some(deadline) = deadline {
        use std::sync::mpsc::channel;
        use std::sync::{Arc, Condvar, Mutex};
        use std::thread;
        let master_signal = Arc::new((Mutex::new(false), Condvar::new()));
        let slave_signal = master_signal.clone();
        let (tx, rx) = channel();
        thread::spawn(move || {
            let (lock, cvar) = &*slave_signal;
            if tx // try to get a socks stream and send it to the parent thread's rx
                .send(if proto == Proto::SOCKS5 {
                    get_socks5_stream(&proxy, &proxy_addr, host_addr)
                } else {
                    get_socks4_stream(&proxy_addr, host_addr)
                })
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

        let timeout_connect = time_until_deadline(deadline)?;
        let done_result = cvar.wait_timeout(done, timeout_connect).unwrap();
        let done = done_result.0;
        if *done {
            rx.recv().unwrap()?
        } else {
            return Err(io_err_timeout(format!(
                "SOCKS proxy: {}:{} timed out connecting after {}ms.",
                host,
                port,
                timeout_connect.as_millis()
            )));
        }
    } else if proto == Proto::SOCKS5 {
        get_socks5_stream(&proxy, &proxy_addr, host_addr)?
    } else {
        get_socks4_stream(&proxy_addr, host_addr)?
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
            proxy.user.as_ref().unwrap(),
            proxy.password.as_ref().unwrap(),
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

#[cfg(feature = "socks-proxy")]
fn get_socks4_stream(
    proxy_addr: &SocketAddr,
    host_addr: TargetAddr,
) -> Result<TcpStream, std::io::Error> {
    match socks::Socks4Stream::connect(proxy_addr, host_addr, "") {
        Ok(socks_stream) => Ok(socks_stream.into_inner()),
        Err(err) => Err(err),
    }
}

#[cfg(not(feature = "socks-proxy"))]
fn connect_socks(
    _unit: &Unit,
    _proxy: Proxy,
    _deadline: Option<Instant>,
    _proxy_addr: SocketAddr,
    _hostname: &str,
    _port: u16,
    _proto: Proto,
) -> Result<TcpStream, std::io::Error> {
    Err(std::io::Error::new(
        io::ErrorKind::Other,
        "SOCKS feature disabled.",
    ))
}

#[cfg(test)]
pub(crate) fn connect_test(unit: &Unit) -> Result<Stream, Error> {
    use crate::test;
    test::resolve_handler(unit)
}

#[cfg(not(test))]
pub(crate) fn connect_test(unit: &Unit) -> Result<Stream, Error> {
    Err(ErrorKind::UnknownScheme.msg(format!("unknown scheme '{}'", unit.url.scheme())))
}
