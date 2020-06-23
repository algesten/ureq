#[cfg(any(feature = "tls", feature = "native-tls"))]
use std::io::Read;

#[cfg(any(feature = "tls", feature = "native-tls"))]
use super::super::*;

#[test]
#[cfg(any(feature = "tls", feature = "native-tls"))]
fn read_range() {
    let resp = get("https://ureq.s3.eu-central-1.amazonaws.com/sherlock.txt")
        .set("Range", "bytes=1000-1999")
        .call();
    assert_eq!(resp.status(), 206);
    let mut reader = resp.into_reader();
    let mut buf = vec![];
    let len = reader.read_to_end(&mut buf).unwrap();
    assert_eq!(len, 1000);
    assert_eq!(
        &buf[0..20],
        [83, 99, 111, 116, 116, 34, 10, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32]
    )
}
