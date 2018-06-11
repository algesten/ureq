use rustls;
use std::io::Read;
use std::io::Result;
use std::io::Write;
use std::net::TcpStream;

pub enum Stream {
    Http(TcpStream),
    Https(rustls::ClientSession, TcpStream),
    #[cfg(test)] Test(Box<Read + Send>, Box<Write + Send>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Stream::Http(sock) => sock.read(buf),
            Stream::Https(sess, sock) => rustls::Stream::new(sess, sock).read(buf),
            #[cfg(test)] Stream::Test(reader, _) => reader.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        match self {
            Stream::Http(sock) => sock.write(buf),
            Stream::Https(sess, sock) => rustls::Stream::new(sess, sock).write(buf),
            #[cfg(test)] Stream::Test(_, writer) => writer.write(buf),
        }
    }
    fn flush(&mut self) -> Result<()> {
        match self {
            Stream::Http(sock) => sock.flush(),
            Stream::Https(sess, sock) => rustls::Stream::new(sess, sock).flush(),
            #[cfg(test)] Stream::Test(_, writer) => writer.flush(),
        }
    }
}
