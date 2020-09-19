#![allow(dead_code)]

use crate::test;
use crate::test::testserver::{read_headers, TestServer};
use std::io::{self, Write};
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

    let resp = agent.get("test://host/agent_reuse_headers").call().unwrap();
    assert_eq!(resp.header("X-Call").unwrap(), "1");

    test::set_handler("/agent_reuse_headers", |unit| {
        assert!(unit.has("Authorization"));
        assert_eq!(unit.header("Authorization").unwrap(), "Foo 12345");
        test::make_response(200, "OK", vec!["X-Call: 2"], vec![])
    });

    let resp = agent.get("test://host/agent_reuse_headers").call().unwrap();
    assert_eq!(resp.header("X-Call").unwrap(), "2");
}

#[cfg(feature = "cookie")]
#[test]
fn agent_cookies() {
    let agent = agent();

    test::set_handler("/agent_cookies", |_unit| {
        test::make_response(
            200,
            "OK",
            vec!["Set-Cookie: foo=bar%20baz; Path=/; HttpOnly"],
            vec![],
        )
    });

    agent.get("test://host/agent_cookies").call().unwrap();

    assert!(agent.cookie("foo").is_some());
    assert_eq!(agent.cookie("foo").unwrap().value(), "bar baz");

    test::set_handler("/agent_cookies", |unit| {
        assert!(unit.has("cookie"));
        assert_eq!(unit.header("cookie").unwrap(), "foo=bar%20baz");
        test::make_response(200, "OK", vec![], vec![])
    });

    agent.get("test://host/agent_cookies").call().unwrap();
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
    let resp = agent.get(&url).call().unwrap();

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
    let resp = agent.get(&url).call().unwrap();
    assert_eq!(resp.status(), 200);
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
        },
        "/second" => {
            if headers.headers().iter().find(|&x| x.contains("first=true")).is_none() {
                panic!("request did not contain cookie 'first'");
            }
            stream.write_all(b"HTTP/1.1 302 Found\r\n")?;
            stream.write_all(b"Location: /third\r\n")?;
            stream.write_all(b"Set-Cookie: second=true\r\n")?;
            stream.write_all(b"Content-Length: 0\r\n\r\n")?;
        },
        "/third" => {
            if headers.headers().iter().find(|&x| x.contains("second=true")).is_none() {
                panic!("request did not contain cookie 'second'");
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\n")?;
            stream.write_all(b"Set-Cookie: third=true\r\n")?;
            stream.write_all(b"Content-Length: 0\r\n\r\n")?;
        },
        _ => {},
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
