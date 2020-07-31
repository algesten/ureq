use crate::test::testserver::*;
use std::io::{self, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use super::super::*;

// Send an HTTP response on the TcpStream at a rate of two bytes every 10
// milliseconds, for a total of 600 bytes.
fn dribble_body_respond(mut stream: TcpStream, contents: &[u8]) -> io::Result<()> {
    read_headers(&stream);
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        contents.len() * 2
    );
    stream.write_all(headers.as_bytes())?;
    for i in 0..contents.len() {
        stream.write_all(&contents[i..i + 1])?;
        stream.write_all(&[b'\n'; 1])?;
        stream.flush()?;
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn get_and_expect_timeout(url: String) {
    let agent = Agent::default().build();
    let timeout = Duration::from_millis(500);
    let resp = agent.get(&url).timeout(timeout).call();

    match resp.into_string() {
        Err(io_error) => match io_error.kind() {
            io::ErrorKind::TimedOut => Ok(()),
            _ => Err(format!("{:?}", io_error)),
        },
        Ok(_) => Err("successful response".to_string()),
    }
    .expect("expected timeout but got something else");
}

#[test]
fn overall_timeout_during_body() {
    // Start a test server on an available port, that dribbles out a response at 1 write per 10ms.
    let server = TestServer::new(|stream| dribble_body_respond(stream, &[b'a'; 300]));
    let url = format!("http://localhost:{}/", server.port);
    get_and_expect_timeout(url);
}

// Send HTTP headers on the TcpStream at a rate of one header every 100
// milliseconds, for a total of 30 headers.
fn dribble_headers_respond(mut stream: TcpStream) -> io::Result<()> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n")?;
    for _ in 0..30 {
        stream.write_all(b"a: b\n")?;
        stream.flush()?;
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[test]
fn overall_timeout_during_headers() {
    // Start a test server on an available port, that dribbles out a response at 1 write per 10ms.
    let server = TestServer::new(dribble_headers_respond);
    let url = format!("http://localhost:{}/", server.port);
    get_and_expect_timeout(url);
}

#[test]
#[cfg(feature = "json")]
fn overall_timeout_reading_json() {
    // Start a test server on an available port, that dribbles out a response at 1 write per 10ms.
    let server = TestServer::new(|stream| {
        dribble_body_respond(
            stream,
            b"[1,1,1,1,1,1,1,1,1,1,1,1,1,
        1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
        1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
        1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
        1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]",
        )
    });
    let url = format!("http://localhost:{}/", server.port);

    let agent = Agent::default().build();
    let timeout = Duration::from_millis(500);
    let resp = agent.get(&url).timeout(timeout).call();

    match resp.into_json() {
        Ok(_) => Err("successful response".to_string()),
        Err(e) => match e.kind() {
            io::ErrorKind::TimedOut => Ok(()),
            _ => Err(format!("Unexpected io::ErrorKind: {:?}", e)),
        },
    }
    .expect("expected timeout but got something else");
}
