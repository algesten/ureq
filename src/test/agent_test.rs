#![allow(dead_code)]

use crate::test;
use crate::test::testserver::{read_headers, TestServer};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::super::*;

#[test]
fn agent_reuse_headers() {
    let agent = agent().set("Authorization", "Foo 12345").build();

    test::set_handler("/agent_reuse_headers", |unit| {
        assert!(unit.has("Authorization"));
        assert_eq!(unit.header("Authorization").unwrap(), "Foo 12345");
        test::make_response(200, "OK", vec!["X-Call: 1"], vec![])
    });

    let resp = agent.get("test://host/agent_reuse_headers").call();
    assert_eq!(resp.header("X-Call").unwrap(), "1");

    test::set_handler("/agent_reuse_headers", |unit| {
        assert!(unit.has("Authorization"));
        assert_eq!(unit.header("Authorization").unwrap(), "Foo 12345");
        test::make_response(200, "OK", vec!["X-Call: 2"], vec![])
    });

    let resp = agent.get("test://host/agent_reuse_headers").call();
    assert_eq!(resp.header("X-Call").unwrap(), "2");
}

// Handler that answers with a simple HTTP response, and times
// out idle connections after 2 seconds.
fn idle_timeout_handler(mut stream: TcpStream) -> io::Result<()> {
    read_headers(&stream);
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nresponse")?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    Ok(())
}

#[test]
fn connection_reuse() {
    let testserver = TestServer::new(idle_timeout_handler);
    let url = format!("http://localhost:{}", testserver.port);
    let agent = Agent::default().build();
    let resp = agent.get(&url).call();

    // use up the connection so it gets returned to the pool
    assert_eq!(resp.status(), 200);
    resp.into_string().unwrap();

    {
        let mut state = agent.state.lock().unwrap();
        assert!(state.pool().len() > 0);
    }

    // wait for the server to close the connection.
    std::thread::sleep(Duration::from_secs(3));

    // try and make a new request on the pool. this fails
    // when we discover that the TLS connection is dead
    // first when attempting to read from it.
    // Note: This test assumes the second  .call() actually
    // pulls from the pool. If for some reason the timed-out
    // connection wasn't in the pool, we won't be testing what
    // we thought we were testing.
    let resp = agent.get(&url).call();
    if let Some(err) = resp.synthetic_error() {
        panic!("Pooled connection failed! {:?}", err);
    }
    assert_eq!(resp.status(), 200);
}

#[test]
fn custom_resolver() {
    use std::io::Read;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let local_addr = listener.local_addr().unwrap();

    let server = std::thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 16];
        let read = client.read(&mut buf).unwrap();
        buf.truncate(read);
        buf
    });

    crate::agent()
        .set_resolver(move |_: &str| Ok(vec![local_addr]))
        .get("http://cool.server/")
        .call();

    assert_eq!(&server.join().unwrap(), b"GET / HTTP/1.1\r\n");
}

#[cfg(feature = "cookie")]
#[cfg(test)]
fn cookie_and_redirect(mut stream: TcpStream) -> io::Result<()> {
    let headers = read_headers(&stream);
    match headers.path() {
        "/first" => {
            stream.write_all(b"HTTP/1.1 302 Found\r\n")?;
            stream.write_all(b"Location: /second\r\n")?;
            stream.write_all(b"Set-Cookie: first=true\r\n")?;
            stream.write_all(b"Content-Length: 0\r\n\r\n")?;
        }
        "/second" => {
            if headers
                .headers()
                .iter()
                .find(|&x| x.contains("first=true"))
                .is_none()
            {
                panic!("request did not contain cookie 'first'");
            }
            stream.write_all(b"HTTP/1.1 302 Found\r\n")?;
            stream.write_all(b"Location: /third\r\n")?;
            stream.write_all(b"Set-Cookie: second=true\r\n")?;
            stream.write_all(b"Content-Length: 0\r\n\r\n")?;
        }
        "/third" => {
            if headers
                .headers()
                .iter()
                .find(|&x| x.contains("second=true"))
                .is_none()
            {
                panic!("request did not contain cookie 'second'");
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\n")?;
            stream.write_all(b"Set-Cookie: third=true\r\n")?;
            stream.write_all(b"Content-Length: 0\r\n\r\n")?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(feature = "cookie")]
#[test]
fn test_cookies_on_redirect() {
    let testserver = TestServer::new(cookie_and_redirect);
    let url = format!("http://localhost:{}/first", testserver.port);
    let agent = Agent::default().build();
    let resp = agent.post(&url).call();
    if resp.error() {
        panic!("error: {} {}", resp.status(), resp.into_string().unwrap());
    }
    assert!(agent.cookie("first").is_some());
    assert!(agent.cookie("second").is_some());
    assert!(agent.cookie("third").is_some());
}

#[test]
fn dirty_streams_not_returned() -> io::Result<()> {
    let testserver = TestServer::new(|mut stream: TcpStream| -> io::Result<()> {
        read_headers(&stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\n")?;
        stream.write_all(b"Transfer-Encoding: chunked\r\n")?;
        stream.write_all(b"\r\n")?;
        stream.write_all(b"5\r\n")?;
        stream.write_all(b"corgi\r\n")?;
        stream.write_all(b"8\r\n")?;
        stream.write_all(b"dachsund\r\n")?;
        stream.write_all(b"0\r\n")?;
        stream.write_all(b"\r\n")?;
        Ok(())
    });
    let url = format!("http://localhost:{}/", testserver.port);
    let agent = Agent::default().build();
    let resp = agent.get(&url).call();
    if let Some(err) = resp.synthetic_error() {
        panic!("resp failed: {:?}", err);
    }
    let resp_str = resp.into_string()?;
    assert_eq!(resp_str, "corgidachsund");

    // Now fetch it again, but only read part of the body.
    let resp_to_be_dropped = agent.get(&url).call();
    if let Some(err) = resp_to_be_dropped.synthetic_error() {
        panic!("resp_to_be_dropped failed: {:?}", err);
    }
    let mut reader = resp_to_be_dropped.into_reader();

    // Read 9 bytes of the response and then drop the reader.
    let mut buf = [0_u8; 4];
    let n = reader.read(&mut buf)?;
    assert_ne!(n, 0, "early EOF");
    assert_eq!(&buf, b"corg");
    drop(reader);

    let resp_to_succeed = agent.get(&url).call();
    if let Some(err) = resp_to_succeed.synthetic_error() {
        panic!("resp_to_succeed failed: {:?}", err);
    }

    Ok(())
}
