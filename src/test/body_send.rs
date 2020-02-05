use crate::test;

use super::super::*;

#[test]
fn content_length_on_str() {
    test::set_handler("/content_length_on_str", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = post("test://host/content_length_on_str").send_string("Hello World!!!");
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("\r\nContent-Length: 14\r\n"));
}

#[test]
fn user_set_content_length_on_str() {
    test::set_handler("/user_set_content_length_on_str", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = post("test://host/user_set_content_length_on_str")
        .set("Content-Length", "12345")
        .send_string("Hello World!!!");
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("\r\nContent-Length: 12345\r\n"));
}

#[test]
#[cfg(feature = "json")]
fn content_length_on_json() {
    test::set_handler("/content_length_on_json", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let mut json = SerdeMap::new();
    json.insert(
        "Hello".to_string(),
        SerdeValue::String("World!!!".to_string()),
    );
    let resp = post("test://host/content_length_on_json").send_json(SerdeValue::Object(json));
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("\r\nContent-Length: 20\r\n"));
}

#[test]
fn content_length_and_chunked() {
    test::set_handler("/content_length_and_chunked", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = post("test://host/content_length_and_chunked")
        .set("Transfer-Encoding", "chunked")
        .send_string("Hello World!!!");
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("Transfer-Encoding: chunked\r\n"));
    assert!(!s.contains("\r\nContent-Length:\r\n"));
}

#[test]
#[cfg(feature = "charset")]
fn str_with_encoding() {
    test::set_handler("/str_with_encoding", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = post("test://host/str_with_encoding")
        .set("Content-Type", "text/plain; charset=iso-8859-1")
        .send_string("Hällo Wörld!!!");
    let vec = resp.to_write_vec();
    assert_eq!(
        &vec[vec.len() - 14..],
        //H  ä    l    l    o    _   W   ö    r    l    d    !   !   !
        [72, 228, 108, 108, 111, 32, 87, 246, 114, 108, 100, 33, 33, 33]
    );
}

#[test]
#[cfg(feature = "json")]
fn content_type_on_json() {
    test::set_handler("/content_type_on_json", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let mut json = SerdeMap::new();
    json.insert(
        "Hello".to_string(),
        SerdeValue::String("World!!!".to_string()),
    );
    let resp = post("test://host/content_type_on_json").send_json(SerdeValue::Object(json));
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("\r\nContent-Type: application/json\r\n"));
}

#[test]
#[cfg(feature = "json")]
fn content_type_not_overriden_on_json() {
    test::set_handler("/content_type_not_overriden_on_json", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let mut json = SerdeMap::new();
    json.insert(
        "Hello".to_string(),
        SerdeValue::String("World!!!".to_string()),
    );
    let resp = post("test://host/content_type_not_overriden_on_json")
        .set("content-type", "text/plain")
        .send_json(SerdeValue::Object(json));
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("\r\ncontent-type: text/plain\r\n"));
}
