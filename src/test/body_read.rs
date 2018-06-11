use std::io::Read;
use test;

use super::super::*;

#[test]
fn transfer_encoding_bogus() {
    test::set_handler("/transfer_encoding_bogus", |_req, _url| {
        test::make_stream(
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
    test::set_handler("/content_length_limited", |_req, _url| {
        test::make_stream(
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
