use crate::middleware::{Middleware, MiddlewareNext};
use crate::{http, http::HeaderValue, Body, Error, SendBody};
use digest_auth::{AuthContext, AuthorizationHeader, WwwAuthenticateHeader};
use std::str::FromStr;

/// Provides simple digest authentication powered by the `digest_auth` crate.
///
/// Requests that receive a HTTP 401 response are retried once by this middleware with the
/// credentials provided on construction. The retry only happens under these conditions:
/// - there is no prior "authorization" header on the request set by the caller or other
///   middleware, and;
/// - the server provides HTTP Digest auth challenge in the "www-authenticate" header.
/// - the request body is empty (limitation to avoid body consumption issues)
///
/// In other cases, this middleware acts as a no-op forwarder of requests and responses.
///
/// **Note**: This middleware requires [`http_status_as_error(false)`] to be configured
/// on the agent, as it needs to respond to 401's rather than treat them as errors.
///
/// [`http_status_as_error(false)`]: crate::config::ConfigBuilder::http_status_as_error
///
/// ```
/// let arbitrary_username = "MyUsername";
/// let arbitrary_password = "MyPassword";
/// let digest_auth_middleware =
///     ureq::DigestAuthMiddleware::new(arbitrary_username, arbitrary_password);
/// # let url = String::new();
///
/// let agent: ureq::Agent = ureq::config::Config::builder()
///     .http_status_as_error(false)  // Required for digest auth
///     .middleware(digest_auth_middleware)
///     .build()
///     .into();
/// agent.get(&url).call();
/// ```
pub struct DigestAuthMiddleware {
    username: String,
    password: String,
}

impl DigestAuthMiddleware {
    /// Create a new digest authentication middleware.
    pub fn new(username: &str, password: &str) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }

    fn construct_answer_to_challenge(
        &self,
        uri: &http::Uri,
        response: &http::Response<Body>,
    ) -> Option<HeaderValue> {
        let challenge_string = response
            .headers()
            .get(http::header::WWW_AUTHENTICATE)?
            .to_str()
            .ok()?;
        let mut challenge = WwwAuthenticateHeader::from_str(challenge_string).ok()?;

        let path = uri.path();
        let context = AuthContext::new(&self.username, &self.password, path);
        let auth_header: AuthorizationHeader = challenge.respond(&context).ok()?;

        HeaderValue::from_str(&auth_header.to_string()).ok()
    }
}

impl Middleware for DigestAuthMiddleware {
    fn handle(
        &self,
        request: http::Request<SendBody>,
        next: MiddlewareNext,
    ) -> Result<http::Response<Body>, Error> {
        // Prevent infinite recursion when doing a nested request below.
        if request.headers().get(http::header::AUTHORIZATION).is_some() {
            return next.handle(request);
        }

        // Clone for the authentication challenge response.
        let (mut parts, body) = request.into_parts();
        let request = http::Request::from_parts(parts.clone(), body);
        let agent = next.agent;

        let response = next.handle(request)?;

        if response.status() == http::StatusCode::UNAUTHORIZED {
            if let Some(challenge_answer_header) =
                self.construct_answer_to_challenge(&parts.uri, &response)
            {
                parts
                    .headers
                    .insert(http::header::AUTHORIZATION, challenge_answer_header);

                let retry_request = http::Request::from_parts(parts, SendBody::none());
                return agent.run(retry_request);
            }
        }

        Ok(response)
    }
}
