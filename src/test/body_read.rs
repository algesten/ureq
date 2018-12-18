use crate::test;
use std::io::Read;

use super::super::*;

#[test]
fn transfer_encoding_bogus() {
    test::set_handler("/transfer_encoding_bogus", |_unit| {
        test::make_response(
            200,
            "OK",
            vec![
                "transfer-encoding: bogus", // whatever it says here, we should chunk
            ],
            "3\r\nhel\r\nb\r\nlo world!!!\r\n0\r\n\r\n"
                .to_string()
                .into_bytes(),
        )
    });
    let resp = get("test://host/transfer_encoding_bogus").call();
    let mut reader = resp.into_reader();
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "hello world!!!");
}

#[test]
fn content_length_limited() {
    test::set_handler("/content_length_limited", |_unit| {
        test::make_response(
            200,
            "OK",
            vec!["Content-Length: 4"],
            "abcdefgh".to_string().into_bytes(),
        )
    });
    let resp = get("test://host/content_length_limited").call();
    let mut reader = resp.into_reader();
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "abcd");
}

#[test]
// content-length should be ignored when chunked
fn ignore_content_length_when_chunked() {
    test::set_handler("/ignore_content_length_when_chunked", |_unit| {
        test::make_response(
            200,
            "OK",
            vec!["Content-Length: 4", "transfer-encoding: chunked"],
            "3\r\nhel\r\nb\r\nlo world!!!\r\n0\r\n\r\n"
                .to_string()
                .into_bytes(),
        )
    });
    let resp = get("test://host/ignore_content_length_when_chunked").call();
    let mut reader = resp.into_reader();
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "hello world!!!");
}

#[test]
fn no_reader_on_head() {
    test::set_handler("/no_reader_on_head", |_unit| {
        // so this is technically illegal, we return a body for the HEAD request.
        test::make_response(
            200,
            "OK",
            vec!["Content-Length: 4", "transfer-encoding: chunked"],
            "3\r\nhel\r\nb\r\nlo world!!!\r\n0\r\n\r\n"
                .to_string()
                .into_bytes(),
        )
    });
    let resp = head("test://host/no_reader_on_head").call();
    let mut reader = resp.into_reader();
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "");
}
