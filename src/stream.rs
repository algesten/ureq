use std::io::Read;
use std::io::Result;
use std::io::Write;
use std::net::TcpStream;
use native_tls::TlsStream;


pub enum Stream {
    Http(TcpStream),
    Https(TlsStream<TcpStream>),
    Read(Box<Read>),
    #[cfg(test)] Test(Box<Read + Send>, Vec<u8>),
}

impl Stream {
    #[cfg(test)]
    pub fn to_write_vec(&self) -> Vec<u8> {
        match self {
            Stream::Test(_, writer) => writer.clone(),
            _ => panic!("to_write_vec on non Test stream")
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Stream::Http(sock) => sock.read(buf),
            Stream::Https(stream) => stream.read(buf),
            Stream::Read(read) => read.read(buf),
            #[cfg(test)] Stream::Test(reader, _) => reader.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        match self {
            Stream::Http(sock) => sock.write(buf),
            Stream::Https(stream) => stream.write(buf),
            Stream::Read(_) => panic!("Write to read stream"),
            #[cfg(test)] Stream::Test(_, writer) => writer.write(buf),
        }
    }
    fn flush(&mut self) -> Result<()> {
        match self {
            Stream::Http(sock) => sock.flush(),
            Stream::Https(stream) => stream.flush(),
            Stream::Read(_) => panic!("Flush read stream"),
            #[cfg(test)] Stream::Test(_, writer) => writer.flush(),
        }
    }
}
