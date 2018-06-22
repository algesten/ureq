use test;

use super::super::*;

#[test]
fn basic_auth() {
    test::set_handler("/basic_auth", |req, _url| {
        assert_eq!(
            req.header("Authorization").unwrap(),
            "Basic bWFydGluOnJ1YmJlcm1hc2hndW0="
        );
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/basic_auth")
        .auth("martin", "rubbermashgum")
        .call();
    assert_eq!(*resp.status(), 200);
}

#[test]
fn kind_auth() {
    test::set_handler("/kind_auth", |req, _url| {
        assert_eq!(req.header("Authorization").unwrap(), "Digest abcdefgh123");
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/kind_auth")
        .auth_kind("Digest", "abcdefgh123")
        .call();
    assert_eq!(*resp.status(), 200);
}
