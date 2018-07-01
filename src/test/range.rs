use std::io::Read;

use super::super::*;

#[test]
fn read_range() {
    let resp = get("https://s3.amazonaws.com/foosrvr/bbb.mp4")
        .set("Range", "bytes=1000-1999")
        .call();
    assert_eq!(resp.status(), 206);
    let mut reader = resp.into_reader();
    let mut buf = vec![];
    let len = reader.read_to_end(&mut buf).unwrap();
    assert_eq!(len, 1000);
    assert_eq!(
        &buf[0..20],
        [0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 3, 232, 0, 0, 0, 1]
    )
}

#[test]
fn agent_pool() {
    let agent = agent().build();

    // req 1
    let resp = agent.get("https://s3.amazonaws.com/foosrvr/bbb.mp4")
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
        let foo = format!("{:?}", pool.get("s3.amazonaws.com", 443));
        assert_eq!(foo, "Some(Stream[https])"); // not a great way of testing.
    }

    // req 2 should be done with a reused connection
    let resp = agent.get("https://s3.amazonaws.com/foosrvr/bbb.mp4")
        .set("Range", "bytes=5000-6999")
        .call();
    assert_eq!(resp.status(), 206);
    let mut reader = resp.into_reader();
    let mut buf = vec![];
    let len = reader.read_to_end(&mut buf).unwrap();
    assert_eq!(len, 2000);
}
