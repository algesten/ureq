use std::io::Read;
use std::net::TcpListener;

#[test]
fn adresses_overridden() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let listener_addr = listener.local_addr().unwrap();

    let server = std::thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 16];
        let read = client.read(&mut buf).unwrap();
        buf.truncate(read);
        buf
    });

    ureq::get("http://cool.server/")
        .set_addresses(vec![listener_addr])
        .call();

    assert_eq!(&server.join().unwrap(), b"GET / HTTP/1.1\r\n");
}
