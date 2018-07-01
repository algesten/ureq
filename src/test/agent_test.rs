use test;

use super::super::*;

#[test]
fn agent_reuse_headers() {
    let agent = agent().set("Authorization", "Foo 12345").build();

    test::set_handler("/agent_reuse_headers", |unit| {
        assert!(unit.has("Authorization"));
        assert_eq!(unit.header("Authorization").unwrap(), "Foo 12345");
        test::make_response(200, "OK", vec!["X-Call: 1"], vec![])
    });

    let resp = agent.get("test://host/agent_reuse_headers").call();
    assert_eq!(resp.header("X-Call").unwrap(), "1");

    test::set_handler("/agent_reuse_headers", |unit| {
        assert!(unit.has("Authorization"));
        assert_eq!(unit.header("Authorization").unwrap(), "Foo 12345");
        test::make_response(200, "OK", vec!["X-Call: 2"], vec![])
    });

    let resp = agent.get("test://host/agent_reuse_headers").call();
    assert_eq!(resp.header("X-Call").unwrap(), "2");
}

#[test]
fn agent_cookies() {
    let agent = agent();

    test::set_handler("/agent_cookies", |_unit| {
        test::make_response(
            200,
            "OK",
            vec!["Set-Cookie: foo=bar%20baz; Path=/; HttpOnly"],
            vec![],
        )
    });

    agent.get("test://host/agent_cookies").call();

    assert!(agent.cookie("foo").is_some());
    assert_eq!(agent.cookie("foo").unwrap().value(), "bar baz");

    test::set_handler("/agent_cookies", |unit| {
        assert!(unit.has("cookie"));
        assert_eq!(unit.header("cookie").unwrap(), "foo=bar%20baz");
        test::make_response(200, "OK", vec![], vec![])
    });

    agent.get("test://host/agent_cookies").call();
}
