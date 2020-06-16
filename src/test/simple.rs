use crate::test;
use std::io::Read;

use super::super::*;

#[test]
fn header_passing() {
    test::set_handler("/header_passing", |unit| {
        assert!(unit.has("X-Foo"));
        assert_eq!(unit.header("X-Foo").unwrap(), "bar");
        test::make_response(200, "OK", vec!["X-Bar: foo"], vec![])
    });
    let resp = get("test://host/header_passing").set("X-Foo", "bar").call();
    assert_eq!(resp.status(), 200);
    assert!(resp.has("X-Bar"));
    assert_eq!(resp.header("X-Bar").unwrap(), "foo");
}

#[test]
fn repeat_non_x_header() {
    test::set_handler("/repeat_non_x_header", |unit| {
        assert!(unit.has("Accept"));
        assert_eq!(unit.header("Accept").unwrap(), "baz");
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/repeat_non_x_header")
        .set("Accept", "bar")
        .set("Accept", "baz")
        .call();
    assert_eq!(resp.status(), 200);
}

#[test]
fn repeat_x_header() {
    test::set_handler("/repeat_x_header", |unit| {
        assert!(unit.has("X-Forwarded-For"));
        assert_eq!(unit.header("X-Forwarded-For").unwrap(), "130.240.19.2");
        assert_eq!(
            unit.all("X-Forwarded-For"),
            vec!["130.240.19.2", "130.240.19.3"]
        );
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/repeat_x_header")
        .set("X-Forwarded-For", "130.240.19.2")
        .set("X-Forwarded-For", "130.240.19.3")
        .call();
    assert_eq!(resp.status(), 200);
}

#[test]
fn body_as_text() {
    test::set_handler("/body_as_text", |_unit| {
        test::make_response(200, "OK", vec![], "Hello World!".to_string().into_bytes())
    });
    let resp = get("test://host/body_as_text").call();
    let text = resp.into_string().unwrap();
    assert_eq!(text, "Hello World!");
}

#[test]
#[cfg(feature = "json")]
fn body_as_json() {
    test::set_handler("/body_as_json", |_unit| {
        test::make_response(
            200,
            "OK",
            vec![],
            "{\"hello\":\"world\"}".to_string().into_bytes(),
        )
    });
    let resp = get("test://host/body_as_json").call();
    let json = resp.into_json().unwrap();
    assert_eq!(json["hello"], "world");
}

#[test]
#[cfg(feature = "json")]
fn body_as_json_deserialize() {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Hello {
        hello: String,
    }

    test::set_handler("/body_as_json_deserialize", |_unit| {
        test::make_response(
            200,
            "OK",
            vec![],
            "{\"hello\":\"world\"}".to_string().into_bytes(),
        )
    });
    let resp = get("test://host/body_as_json_deserialize").call();
    let json = resp.into_json_deserialize::<Hello>().unwrap();
    assert_eq!(json.hello, "world");
}

#[test]
fn body_as_reader() {
    test::set_handler("/body_as_reader", |_unit| {
        test::make_response(200, "OK", vec![], "abcdefgh".to_string().into_bytes())
    });
    let resp = get("test://host/body_as_reader").call();
    let mut reader = resp.into_reader();
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "abcdefgh");
}

#[test]
fn escape_path() {
    test::set_handler("/escape_path%20here", |_unit| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/escape_path here").call();
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("GET /escape_path%20here HTTP/1.1"))
}

#[test]
fn request_debug() {
    let req = get("/my/page")
        .set("Authorization", "abcdef")
        .set("Content-Length", "1234")
        .set("Content-Type", "application/json")
        .build();

    let s = format!("{:?}", req);

    assert_eq!(
        s,
        "Request(GET /my/page, [Authorization: abcdef, \
         Content-Length: 1234, Content-Type: application/json])"
    );

    let req = get("/my/page?q=z")
        .query("foo", "bar baz")
        .set("Authorization", "abcdef")
        .build();

    let s = format!("{:?}", req);

    assert_eq!(
        s,
        "Request(GET /my/page?q=z&foo=bar%20baz, [Authorization: abcdef])"
    );
}

#[test]
fn non_ascii_header() {
    test::set_handler("/non_ascii_header", |_unit| {
        test::make_response(200, "OK", vec!["Wörse: Hädör"], vec![])
    });
    let resp = get("test://host/non_ascii_header")
        .set("Bäd", "Headör")
        .call();
    // surprisingly, this is ok, because this lib is not about enforcing standards.
    assert!(resp.ok());
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.status_text(), "OK");
}

#[test]
pub fn no_status_text() {
    // this one doesn't return the status text
    // let resp = get("https://www.okex.com/api/spot/v3/products")
    test::set_handler("/no_status_text", |_unit| {
        test::make_response(200, "", vec![], vec![])
    });
    let resp = get("test://host/no_status_text").call();
    assert!(resp.ok());
    assert_eq!(resp.status(), 200);
}

#[test]
pub fn header_with_spaces_before_value() {
    test::set_handler("/space_before_value", |unit| {
        assert!(unit.has("X-Test"));
        assert_eq!(unit.header("X-Test").unwrap(), "value");
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://host/space_before_value")
        .set("X-Test", "     value")
        .call();
    assert_eq!(resp.status(), 200);
}

#[test]
pub fn host_no_port() {
    test::set_handler("/host_no_port", |_| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://myhost/host_no_port").call();
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("\r\nHost: myhost\r\n"));
}

#[test]
pub fn host_with_port() {
    test::set_handler("/host_with_port", |_| {
        test::make_response(200, "OK", vec![], vec![])
    });
    let resp = get("test://myhost:234/host_with_port").call();
    let vec = resp.to_write_vec();
    let s = String::from_utf8_lossy(&vec);
    assert!(s.contains("\r\nHost: myhost:234\r\n"));
}
