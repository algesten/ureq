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

#[test]
#[cfg(any(feature = "tls", feature = "native-tls"))]
fn agent_pool() {
    let agent = agent();

    // req 1
    let resp = agent
        .get("https://ureq.s3.eu-central-1.amazonaws.com/sherlock.txt")
        .set("Range", "bytes=1000-1999")
        .call();
    assert_eq!(resp.status(), 206);
    let mut reader = resp.into_reader();
    let mut buf = vec![];
    // reading the entire content will return the connection to the pool
    let len = reader.read_to_end(&mut buf).unwrap();
    assert_eq!(len, 1000);

    {
        let mut lock = agent.state().lock().unwrap();
        let state = lock.as_mut().unwrap();
        let pool = state.pool();
        assert_eq!(pool.len(), 1);
        let f = format!("{:?}", pool.get("ureq.s3.eu-central-1.amazonaws.com", 443));
        assert_eq!(f, "Some(Stream[https])"); // not a great way of testing.
    }

    // req 2 should be done with a reused connection
    let resp = agent
        .get("https://ureq.s3.eu-central-1.amazonaws.com/sherlock.txt")
        .set("Range", "bytes=5000-6999")
        .call();
    assert_eq!(resp.status(), 206);
    let mut reader = resp.into_reader();
    let mut buf = vec![];
    let len = reader.read_to_end(&mut buf).unwrap();
    assert_eq!(len, 2000);
}
