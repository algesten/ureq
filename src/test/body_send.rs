use test;

use super::super::*;

#[test]
fn content_length_on_str() {
    test::set_handler("/content_length_on_str", |_req, _url| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = post("test://host/content_length_on_str").send_string("Hello World!!!");
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("\r\nContent-Length: 14\r\n"));
}

#[test]
fn user_set_content_length_on_str() {
    test::set_handler("/user_set_content_length_on_str", |_req, _url| {
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
fn content_length_on_json() {
    test::set_handler("/content_length_on_json", |_req, _url| {
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
    test::set_handler("/content_length_and_chunked", |_req, _url| {
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
