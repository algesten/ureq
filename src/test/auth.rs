use crate::test;

use super::super::*;

#[test]
fn basic_auth() {
    test::set_handler("/basic_auth", |unit| {
        assert_eq!(
            unit.header("Authorization").unwrap(),
            "Basic bWFydGluOnJ1YmJlcm1hc2hndW0="
        );
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/basic_auth")
        .auth("martin", "rubbermashgum")
        .call();
    assert_eq!(resp.status(), 200);
}

#[test]
fn kind_auth() {
    test::set_handler("/kind_auth", |unit| {
        assert_eq!(unit.header("Authorization").unwrap(), "Digest abcdefgh123");
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/kind_auth")
        .auth_kind("Digest", "abcdefgh123")
        .call();
    assert_eq!(resp.status(), 200);
}

#[test]
fn url_auth() {
    test::set_handler("/url_auth", |unit| {
        assert_eq!(
            unit.header("Authorization").unwrap(),
            "Basic QWxhZGRpbjpPcGVuU2VzYW1l"
        );
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://Aladdin:OpenSesame@host/url_auth").call();
    assert_eq!(resp.status(), 200);
}

#[test]
fn url_auth_overridden() {
    test::set_handler("/url_auth_overridden", |unit| {
        assert_eq!(
            unit.header("Authorization").unwrap(),
            "Basic bWFydGluOnJ1YmJlcm1hc2hndW0="
        );
        test::make_response(200, "OK", vec![], vec![])
    });
    let agent = agent().auth("martin", "rubbermashgum").build();
    let resp = agent
        .get("test://Aladdin:OpenSesame@host/url_auth_overridden")
        .call();
    assert_eq!(resp.status(), 200);
}
