use super::super::*;
use test;

#[test]
fn header_passing() {
    test::set_handler("/header_passing", |req, _url| {
        assert!(req.has("X-Foo"));
        assert_eq!(req.get("X-Foo").unwrap(), "bar");
        test::make_stream(200, "OK", vec!["X-Bar: foo"], vec![])
    });
    let resp = get("test://host/header_passing").set("X-Foo", "bar").call();
    assert_eq!(*resp.status(), 200);
    assert!(resp.has("X-Bar"));
    assert_eq!(resp.get("X-Bar").unwrap(), "foo");
}

#[test]
fn body_as_text() {
    test::set_handler("/body_as_text", |_req, _url| {
        test::make_stream(200, "OK", vec![], "Hello World!".to_string().into_bytes())
    });
    let resp = get("test://host/body_as_text").call();
    let text = resp.into_string().unwrap();
    assert_eq!(text, "Hello World!");
}
