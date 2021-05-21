use crate::test;

use super::super::*;

#[test]
fn no_query_string() {
    test::set_handler("/no_query_string", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/no_query_string").call().unwrap();
    let vec = resp.as_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("GET /no_query_string HTTP/1.1"))
}

#[test]
fn escaped_query_string() {
    test::set_handler("/escaped_query_string", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/escaped_query_string")
        .query("foo", "bar")
        .query("baz", "yo lo")
        .call()
        .unwrap();
    let vec = resp.as_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(
        s.contains("GET /escaped_query_string?foo=bar&baz=yo+lo HTTP/1.1"),
        "req: {}",
        s
    );
}

#[test]
fn query_in_path() {
    test::set_handler("/query_in_path", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/query_in_path?foo=bar").call().unwrap();
    let vec = resp.as_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("GET /query_in_path?foo=bar HTTP/1.1"))
}

#[test]
fn query_in_path_and_req() {
    test::set_handler("/query_in_path_and_req", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/query_in_path_and_req?foo=bar")
        .query("baz", "1 2 3")
        .call()
        .unwrap();
    let vec = resp.as_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("GET /query_in_path_and_req?foo=bar&baz=1+2+3 HTTP/1.1"))
}
