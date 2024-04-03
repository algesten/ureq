use std::time::Duration;

use super::super::*;

#[test]
fn valid_credentials() {
    let arbitrary_username = "MyUsername";
    let arbitrary_password = "MyPassword";
    let digest_auth_middleware = DigestAuthMiddleware::new(arbitrary_username, arbitrary_password);
    let test_url = format!(
        "http://httpbin.org/digest-auth/auth/{}/{}",
        arbitrary_username, arbitrary_password
    );
    let agent = AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .middleware(digest_auth_middleware)
        .build();
    let result = agent.get(&test_url).call();
    assert_eq!(result.unwrap().status(), 200);
}

#[test]
fn invalid_credentials() {
    let arbitrary_username = "MyUsername";
    let arbitrary_password = "MyPassword";
    let bad_password = "BadPassword";
    let digest_auth_middleware = DigestAuthMiddleware::new(arbitrary_username, bad_password);
    let test_url = format!(
        "http://httpbin.org/digest-auth/auth/{}/{}",
        arbitrary_username, arbitrary_password
    );
    let agent = AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .middleware(digest_auth_middleware)
        .build();
    let result = agent.get(&test_url).call();
    assert!(
        matches!(result, Err(Error::Status(401, _))),
        "Expected 401 error, received {:?}",
        result
    );
}
