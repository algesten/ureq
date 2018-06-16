use test;

use super::super::*;

#[test]
fn no_query_string() {
    test::set_handler("/no_query_string", |_req, _url| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/no_query_string")
        .call();
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("GET /no_query_string HTTP/1.1"))
}

#[test]
fn escaped_query_string() {
    test::set_handler("/escaped_query_string", |_req, _url| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/escaped_query_string")
        .query("foo", "bar")
        .query("baz", "yo lo")
        .call();
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("GET /escaped_query_string?foo=bar&baz=yo%20lo HTTP/1.1"))
}
