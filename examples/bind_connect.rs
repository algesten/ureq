use socket2::{Domain, Socket, Type};
use std::net::SocketAddr;
use ureq::Connector;

#[derive(Debug)]
pub(crate) struct BindConnector {
    bind_addr: SocketAddr,
}

impl BindConnector {
    pub fn new_bind(bind_addr: SocketAddr) -> Self {
        Self { bind_addr }
    }
}

impl Connector for BindConnector {
    fn connect(&self, addr: &std::net::SocketAddr) -> std::io::Result<std::net::TcpStream> {
        let socket = Socket::new(Domain::for_address(addr.to_owned()), Type::STREAM, None)?;
        socket.bind(&self.bind_addr.into())?;
        socket.connect(&addr.to_owned().into())?;
        Ok(socket.into())
    }

    fn connect_timeout(
        &self,
        addr: &std::net::SocketAddr,
        timeout: std::time::Duration,
    ) -> std::io::Result<std::net::TcpStream> {
        let socket = Socket::new(Domain::for_address(addr.to_owned()), Type::STREAM, None)?;
        socket.bind(&self.bind_addr.into())?;
        socket.connect_timeout(&addr.to_owned().into(), timeout)?;
        Ok(socket.into())
    }
}

pub fn main() {
    let agent = ureq::builder()
        .connector(BindConnector::new_bind("127.0.0.1:54321".parse().unwrap()))
        .build();

    let result = agent.get("http://127.0.0.1:8080/").call();

    match result {
        Err(err) => {
            println!("{:?}", err);
            std::process::exit(1);
        }
        Ok(response) => {
            assert_eq!(response.status(), 200);
        }
    }
}
