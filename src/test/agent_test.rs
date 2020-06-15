use crate::test;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::thread;
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

// Start a test server on an available port, that times out idle connections at 2 seconds.
// Return the port this server is listening on.
fn start_idle_timeout_server() -> u16 {
    let listener = std::net::TcpListener::bind("localhost:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            thread::spawn(move || {
                let stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut write_stream = stream.try_clone().unwrap();
                for line in BufReader::new(stream).lines() {
                    let line = match line {
                        Ok(x) => x,
                        Err(_) => return,
                    };
                    if line == "" {
                        write_stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nresponse")
                            .unwrap();
                    }
                }
            });
        }
    });
    port
}

#[test]
fn connection_reuse() {
    let port = start_idle_timeout_server();
    let url = format!("http://localhost:{}", port);
    let agent = Agent::default().build();
    let resp = agent.get(&url).call();

    // use up the connection so it gets returned to the pool
    assert_eq!(resp.status(), 200);
    let mut buf = vec![];
    resp.into_reader().read_to_end(&mut buf).unwrap();

    {
        let mut guard_state = agent.state().lock().unwrap();
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

// Send an HTTP response on the TcpStream at a rate of two bytes every 10
// milliseconds, for a total of 600 bytes.
fn dribble_respond(stream: &mut std::net::TcpStream) -> io::Result<()> {
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

// Start a test server on an available port, that dribbles out a response at 1 write per 10ms.
// Return the port this server is listening on.
fn start_dribble_server() -> u16 {
    let listener = std::net::TcpListener::bind("localhost:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dribble_handler = |stream: std::net::TcpStream| {
        let mut write_stream = stream.try_clone().unwrap();
        for line in BufReader::new(stream).lines() {
            let line = match line {
                Ok(x) => x,
                Err(_) => return,
            };
            if line == "" {
                if let Err(e) = dribble_respond(&mut write_stream) {
                    eprintln!("sending dribble repsonse: {}", e);
                }
            }
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
fn overall_timeout_during_body() {
    let port = start_dribble_server();
    let url = format!("http://localhost:{}/", port);

    let agent = Agent::default().build();
    let resp = agent.get(&url).timeout(2000).call();

    let mut reader = resp.into_reader();
    let mut bytes = vec![];
    let result = reader.read_to_end(&mut bytes);

    // assert!(resp.error(), "expected timeout error");
    // assert!(resp.synthetic(), "expected timeout error to be synthetic");
    // match resp.synthetic_error() {
    match result {
        Err(io_error) if io_error.kind() == io::ErrorKind::WouldBlock => {}
        e => assert!(false, "wrong type of result. expected timeout, got {:?}", e),
    }
}
