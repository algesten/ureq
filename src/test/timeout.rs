
use crate::test;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use super::super::*;

// Send an HTTP response on the TcpStream at a rate of two bytes every 10
// milliseconds, for a total of 600 bytes.
fn dribble_body_respond(stream: &mut TcpStream) -> io::Result<()> {
    let contents = [b'a'; 300];
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

// Read a stream until reaching a blank line, in order to consume
// request headers.
fn read_headers(stream: &TcpStream) {
    for line in BufReader::new(stream).lines() {
        let line = match line {
            Ok(x) => x,
            Err(_) => return,
        };
        if line == "" {
            break;
        }
    }
}

// Start a test server on an available port, that dribbles out a response at 1 write per 10ms.
// Return the port this server is listening on.
fn start_dribble_body_server() -> u16 {
    let listener = std::net::TcpListener::bind("localhost:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dribble_handler = |mut stream: TcpStream| {
        read_headers(&stream);
        if let Err(e) = dribble_body_respond(&mut stream) {
            eprintln!("sending dribble repsonse: {}", e);
        }
    };
    thread::spawn(move || {
        for stream in listener.incoming() {
            thread::spawn(move || dribble_handler(stream.unwrap()));
        }
    });
    port
}

fn get_and_expect_timeout(url: String) {
    let agent = Agent::default().build();
    let timeout = Duration::from_millis(500);
    let resp = agent.get(&url).timeout(timeout).call();

    let mut reader = resp.into_reader();
    let mut bytes = vec![];
    let result = reader.read_to_end(&mut bytes);

    match result {
        Err(io_error) => match io_error.kind() {
            io::ErrorKind::WouldBlock => Ok(()),
            io::ErrorKind::TimedOut => Ok(()),
            _ => Err(format!("{:?}", io_error)),
        },
        Ok(_) => Err("successful response".to_string()),
    }
    .expect("expected timeout but got something else");
}

#[test]
fn overall_timeout_during_body() {
    let port = start_dribble_body_server();
    let url = format!("http://localhost:{}/", port);

    get_and_expect_timeout(url);
}

// Send HTTP headers on the TcpStream at a rate of one header every 100
// milliseconds, for a total of 30 headers.
fn dribble_headers_respond(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n")?;
    for _ in 0..30 {
        stream.write_all(b"a: b\n")?;
        stream.flush()?;
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

// Start a test server on an available port, that dribbles out response *headers* at 1 write per 10ms.
// Return the port this server is listening on.
fn start_dribble_headers_server() -> u16 {
    let listener = std::net::TcpListener::bind("localhost:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dribble_handler = |mut stream: TcpStream| {
        read_headers(&stream);
        if let Err(e) = dribble_headers_respond(&mut stream) {
            eprintln!("sending dribble repsonse: {}", e);
        }
    };
    thread::spawn(move || {
        for stream in listener.incoming() {
            thread::spawn(move || dribble_handler(stream.unwrap()));
        }
    });
    port
}

#[test]
fn overall_timeout_during_headers() {
    let port = start_dribble_headers_server();
    let url = format!("http://localhost:{}/", port);
    get_and_expect_timeout(url);
}
