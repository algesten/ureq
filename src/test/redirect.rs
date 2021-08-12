use std::{
    io::{self, Write},
    net::TcpStream,
    thread,
    time::Duration,
};
use testserver::{self, TestServer};

use crate::{error::Error, test};

use super::super::*;

#[test]
fn redirect_on() {
    test::set_handler("/redirect_on1", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_on2"], vec![])
    });
    test::set_handler("/redirect_on2", |_| {
        test::make_response(200, "OK", vec!["x-foo: bar"], vec![])
    });
    let resp = get("test://host/redirect_on1").call().unwrap();
    assert!(resp.has("x-foo"));
    assert_eq!(resp.header("x-foo").unwrap(), "bar");
}

#[test]
fn redirect_many() {
    test::set_handler("/redirect_many1", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_many2"], vec![])
    });
    test::set_handler("/redirect_many2", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_many3"], vec![])
    });
    let result = builder()
        .redirects(1)
        .build()
        .get("test://host/redirect_many1")
        .call();
    assert!(matches!(result, Err(e) if e.kind() == ErrorKind::TooManyRedirects));

    test::set_handler("/redirect_many1", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_many2"], vec![])
    });
    test::set_handler("/redirect_many2", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_many3"], vec![])
    });
    test::set_handler("/redirect_many3", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_many4"], vec![])
    });
    let result = builder()
        .redirects(2)
        .build()
        .get("test://host/redirect_many1")
        .call();
    assert!(matches!(result, Err(e) if e.kind() == ErrorKind::TooManyRedirects));
}

#[test]
fn redirect_off() -> Result<(), Error> {
    test::set_handler("/redirect_off", |_| {
        test::make_response(302, "Go here", vec!["Location: somewhere.else"], vec![])
    });
    let resp = builder()
        .redirects(0)
        .build()
        .get("test://host/redirect_off")
        .call()?;
    assert_eq!(resp.status(), 302);
    assert!(resp.has("Location"));
    assert_eq!(resp.header("Location").unwrap(), "somewhere.else");
    Ok(())
}

#[test]
fn redirect_head() {
    test::set_handler("/redirect_head1", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_head2"], vec![])
    });
    test::set_handler("/redirect_head2", |unit| {
        assert_eq!(unit.method, "HEAD");
        test::make_response(200, "OK", vec!["x-foo: bar"], vec![])
    });
    let resp = head("test://host/redirect_head1").call().unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.get_url(), "test://host/redirect_head2");
    assert!(resp.has("x-foo"));
    assert_eq!(resp.header("x-foo").unwrap(), "bar");
}

#[test]
fn redirect_get() {
    test::set_handler("/redirect_get1", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_get2"], vec![])
    });
    test::set_handler("/redirect_get2", |unit| {
        assert_eq!(unit.method, "GET");
        assert!(unit.has("Range"));
        assert_eq!(unit.header("Range").unwrap(), "bytes=10-50");
        test::make_response(200, "OK", vec!["x-foo: bar"], vec![])
    });
    let resp = get("test://host/redirect_get1")
        .set("Range", "bytes=10-50")
        .call()
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.get_url(), "test://host/redirect_get2");
    assert!(resp.has("x-foo"));
    assert_eq!(resp.header("x-foo").unwrap(), "bar");
}

#[test]
fn redirect_host() {
    // Set up a redirect to a host that doesn't exist; it should fail.
    // TODO: This actually relies on the network for the DNS lookup
    // of example.invalid. We can probably do better by, e.g.
    // overriding the resolver.
    let srv = TestServer::new(|mut stream: TcpStream| -> io::Result<()> {
        testserver::read_request(&stream);
        write!(stream, "HTTP/1.1 302 Found\r\n")?;
        write!(stream, "Location: http://example.invalid/\r\n")?;
        write!(stream, "\r\n")?;
        Ok(())
    });
    let url = format!("http://localhost:{}/", srv.port);
    let result = crate::Agent::new().get(&url).call();
    assert!(
        matches!(result, Err(ref e) if e.kind() == ErrorKind::Dns),
        "expected Err(DnsFailed), got: {:?}",
        result
    );
}

#[test]
fn redirect_post() {
    test::set_handler("/redirect_post1", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_post2"], vec![])
    });
    test::set_handler("/redirect_post2", |unit| {
        assert_eq!(unit.method, "GET");
        test::make_response(200, "OK", vec!["x-foo: bar"], vec![])
    });
    let resp = post("test://host/redirect_post1").call().unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.get_url(), "test://host/redirect_post2");
    assert!(resp.has("x-foo"));
    assert_eq!(resp.header("x-foo").unwrap(), "bar");
}

#[test]
fn redirect_post_with_data() {
    test::set_handler("/redirect_post_d1", |unit| {
        assert_eq!(unit.header("Content-Length").unwrap(), "4");
        test::make_response(302, "Go here", vec!["Location: /redirect_post_d2"], vec![])
    });
    test::set_handler("/redirect_post_d2", |unit| {
        assert_eq!(unit.header("Content-Length"), None);
        assert_eq!(unit.method, "GET");
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = post("test://host/redirect_post_d1")
        .send_string("data")
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[test]
fn redirect_308() {
    test::set_handler("/redirect_get3", |_| {
        test::make_response(308, "Go here", vec!["Location: /valid_response"], vec![])
    });
    test::set_handler("/valid_response", |unit| {
        assert_eq!(unit.method, "GET");
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/redirect_get3").call().unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.get_url(), "test://host/valid_response");
}

#[test]
fn redirects_hit_timeout() {
    // A chain of 2 redirects and an OK, each of which takes 50ms; we set a
    // timeout of 100ms and should error.
    test::set_handler("/redirect_sleep1", |_| {
        thread::sleep(Duration::from_millis(50));
        test::make_response(302, "Go here", vec!["Location: /redirect_sleep2"], vec![])
    });
    test::set_handler("/redirect_sleep2", |_| {
        thread::sleep(Duration::from_millis(50));
        test::make_response(302, "Go here", vec!["Location: /ok"], vec![])
    });
    test::set_handler("/ok", |_| {
        thread::sleep(Duration::from_millis(50));
        test::make_response(200, "Go here", vec![], vec![])
    });
    let req = crate::builder().timeout(Duration::from_millis(100)).build();
    let result = req.get("test://host/redirect_sleep1").call();
    assert!(
        matches!(result, Err(Error::Transport(_))),
        "expected Transport error, got {:?}",
        result
    );
}

#[test]
fn too_many_redirects() {
    for i in 0..10_000 {
        test::set_handler(&format!("/malicious_redirect_{}", i), move |_| {
            let location = format!("Location: /malicious_redirect_{}", i + 1);
            test::make_response(302, "Go here", vec![&location], vec![])
        });
    }

    test::set_handler("/malicious_redirect_10000", |unit| {
        assert_eq!(unit.method, "GET");
        test::make_response(200, "OK", vec![], vec![])
    });

    let req = crate::builder().redirects(10001).build();
    let resp = req.get("test://host/malicious_redirect_0").call().unwrap();
    assert_eq!(resp.get_url(), "test://host/malicious_redirect_10000");
}
