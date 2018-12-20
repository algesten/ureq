use crate::test;

use super::super::*;

#[test]
fn redirect_on() {
    test::set_handler("/redirect_on1", |_| {
        test::make_response(302, "Go here", vec!["Location: /redirect_on2"], vec![])
    });
    test::set_handler("/redirect_on2", |_| {
        test::make_response(200, "OK", vec!["x-foo: bar"], vec![])
    });
    let resp = get("test://host/redirect_on1").call();
    assert_eq!(resp.status(), 200);
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
    let resp = get("test://host/redirect_many1").redirects(1).call();
    assert_eq!(resp.status(), 500);
    assert_eq!(resp.status_text(), "Too Many Redirects");
}

#[test]
fn redirect_off() {
    test::set_handler("/redirect_off", |_| {
        test::make_response(302, "Go here", vec!["Location: somewhere.else"], vec![])
    });
    let resp = get("test://host/redirect_off").redirects(0).call();
    assert_eq!(resp.status(), 302);
    assert!(resp.has("Location"));
    assert_eq!(resp.header("Location").unwrap(), "somewhere.else");
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
    let resp = head("test://host/redirect_head1").call();
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
        .call();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.get_url(), "test://host/redirect_get2");
    assert!(resp.has("x-foo"));
    assert_eq!(resp.header("x-foo").unwrap(), "bar");
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
    let resp = post("test://host/redirect_post1").call();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.get_url(), "test://host/redirect_post2");
    assert!(resp.has("x-foo"));
    assert_eq!(resp.header("x-foo").unwrap(), "bar");
}
