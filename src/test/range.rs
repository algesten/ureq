use std::io::Read;
use test;

use super::super::*;

#[test]
fn read_range() {
    let resp = get("https://s3.amazonaws.com/foosrvr/bbb.mp4")
        .set("Range", "bytes=1000-1999")
        .call();
    assert_eq!(*resp.status(), 206);
    let mut reader = resp.into_reader();
    let mut buf = vec![];
    let len = reader.read_to_end(&mut buf).unwrap();
    assert_eq!(len, 1000);
    assert_eq!(
        &buf[0..20],
        [0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 3, 232, 0, 0, 0, 1]
    )
}
