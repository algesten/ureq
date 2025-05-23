use crate::middleware::{Middleware, MiddlewareNext};
use crate::{http, Body, Error, SendBody};
use digest_auth::{AuthContext, WwwAuthenticateHeader};
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
/// let agent = ureq::config::Config::builder()
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

    fn respond_to_challenge(
        &self,
        uri: &http::Uri,
        response: &http::Response<Body>,
    ) -> Option<String> {
        let challenge_string = response.headers().get("www-authenticate")?.to_str().ok()?;
        let mut challenge = WwwAuthenticateHeader::from_str(challenge_string).ok()?;
        let path = uri.path();
        let context = AuthContext::new(&self.username, &self.password, path);
        challenge
            .respond(&context)
            .as_ref()
            .map(ToString::to_string)
            .ok()
    }
}

impl Middleware for DigestAuthMiddleware {
    fn handle(
        &self,
        request: http::Request<SendBody>,
        next: MiddlewareNext,
    ) -> Result<http::Response<Body>, Error> {
        // Prevent infinite recursion when doing a nested request below.
        if request.headers().get("authorization").is_some() {
            return next.handle(request);
        }

        // Clone for the authentication challenge response.
        let (parts, body) = request.into_parts();
        let request = http::Request::from_parts(parts.clone(), body);
        let agent = next.agent;

        let response = next.handle(request)?;

        if response.status() == 401 {
            if let Some(challenge_answer) = self.respond_to_challenge(&parts.uri, &response) {
                let mut retry_parts = parts;
                retry_parts.headers.insert(
                    "authorization",
                    challenge_answer.parse().map_err(|_| {
                        Error::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "Invalid authorization header",
                        ))
                    })?,
                );

                let retry_request = http::Request::from_parts(retry_parts, SendBody::none());
                return agent.run(retry_request);
            }
        }

        Ok(response)
    }
}
