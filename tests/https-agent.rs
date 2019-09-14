use std::io::Read;

#[cfg(feature = "tls")]
#[test]
fn tls_connection_close() {
    let agent = ureq::Agent::default().build();
    let resp = agent.get("https://example.com/404")
        .set("Connection", "close")
        .call();
    assert_eq!(resp.status(), 404);
    resp.into_reader().read_to_end(&mut vec![]).unwrap();
}
