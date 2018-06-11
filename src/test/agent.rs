use test;

use super::super::*;

#[test]
fn agent_reuse_headers() {
    let agent = agent()
        .set("Authorization", "Foo 12345")
        .build();

    test::set_handler("/agent_reuse_headers", |req, _url| {
        assert!(req.has("Authorization"));
        assert_eq!(req.get("Authorization").unwrap(), "Foo 12345");
        test::make_stream(200, "OK", vec!["X-Call: 1"], vec![])
    });

    let resp = agent.get("test://host/agent_reuse_headers").call();
    assert_eq!(resp.get("X-Call").unwrap(), "1");

    test::set_handler("/agent_reuse_headers", |req, _url| {
        assert!(req.has("Authorization"));
        assert_eq!(req.get("Authorization").unwrap(), "Foo 12345");
        test::make_stream(200, "OK", vec!["X-Call: 2"], vec![])
    });

    let resp = agent.get("test://host/agent_reuse_headers").call();
    assert_eq!(resp.get("X-Call").unwrap(), "2");
}
