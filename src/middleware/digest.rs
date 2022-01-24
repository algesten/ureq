use crate::{Request, Response, Error, Middleware, MiddlewareNext};
use digest_auth::{AuthContext, WwwAuthenticateHeader};
use std::{borrow::Cow, str::FromStr};

/// Provides simple digest authentication powered by the `digest_auth` crate.
///
/// Requests that receive a HTTP 401 response are retried once by this middleware with the
/// credentials provided on construction. The retry only happens under these conditions:
/// - there is no prior "authorization" header on the request set by the caller or other
///   middleware, and;
/// - the server provides HTTP Digest auth challenge in the "www-authenticate" header.
///
/// In other cases, this middleware acts as a no-op forwarder of requests and responses.
///
/// ```
/// let arbitrary_username = "MyUsername";
/// let arbitrary_password = "MyPassword";
/// let digest_auth_middleware =
///     ureq::DigestAuthMiddleware::new(arbitrary_username, arbitrary_password);
/// # let url = String::new();
///
/// let agent = ureq::AgentBuilder::new().middleware(digest_auth_middleware).build();
/// agent.get(&url).call();
/// ```
pub struct DigestAuthMiddleware {
    username: Cow<'static, str>,
    password: Cow<'static, str>,
}

impl DigestAuthMiddleware {
    pub fn new(
        username: impl Into<Cow<'static, str>>,
        password: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }

    fn construct_answer_to_challenge(
        &self,
        request: &Request,
        response: &Response,
    ) -> Option<String> {
        let challenge_string = response.header("www-authenticate")?;
        let mut challenge = WwwAuthenticateHeader::from_str(challenge_string).ok()?;
        let path = request.request_url().ok()?.path().to_string();
        let context = AuthContext::new(
            self.username.as_ref(),
            self.password.as_ref(),
            Cow::from(path),
        );
        challenge
            .respond(&context)
            .as_ref()
            .map(ToString::to_string)
            .ok()
    }
}

impl Middleware for DigestAuthMiddleware {
    fn handle(&self, request: Request, next: MiddlewareNext) -> Result<Response, Error> {
        // Prevent infinite recursion when doing a nested request below.
        if request.header("authorization").is_some() {
            return next.handle(request);
        }

        let response = next.handle(request.clone())?;
        if let (401, Some(challenge_answer)) = (
            response.status(),
            self.construct_answer_to_challenge(&request, &response),
        ) {
            request.set("authorization", &challenge_answer).call()
        } else {
            Ok(response)
        }
    }
}

