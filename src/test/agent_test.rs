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

    agent.get("test://host/agent_cookies").call();

    assert!(agent.cookie("foo").is_some());
    assert_eq!(agent.cookie("foo").unwrap().value(), "bar baz");

    test::set_handler("/agent_cookies", |unit| {
        assert!(unit.has("cookie"));
        assert_eq!(unit.header("cookie").unwrap(), "foo=bar%20baz");
        test::make_response(200, "OK", vec![], vec![])
    });

    agent.get("test://host/agent_cookies").call();
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
        let mut guard_state = agent.state.lock().unwrap();
        let mut state = guard_state.take().unwrap();
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
