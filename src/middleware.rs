use crate::{Error, Request, Response};

/// Chained processing of request (and response).
///
/// # Middleware as `fn`
///
/// The middleware trait is implemented for all functions that have the signature
///
/// `Fn(Request, MiddlewareNext) -> Result<Response, Error>`
///
/// That means the easiest way to implement middleware is by providing a `fn`, like so
///
/// ```no_run
/// # use ureq::{Request, Response, MiddlewareNext, Error};
/// fn my_middleware(req: Request, next: MiddlewareNext) -> Result<Response, Error> {
///     // do middleware things
///
///     // continue the middleware chain
///     next.handle(req)
/// }
/// ```
///
/// # Adding headers
///
/// A common use case is to add headers to the outgoing request. Here an example of how.
///
/// ```
/// # #[cfg(feature = "json")]
/// # fn main() -> Result<(), ureq::Error> {
/// # use ureq::{Request, Response, MiddlewareNext, Error};
/// # ureq::is_test(true);
/// fn my_middleware(req: Request, next: MiddlewareNext) -> Result<Response, Error> {
///     // set my bespoke header and continue the chain
///     next.handle(req.set("X-My-Header", "value_42"))
/// }
///
/// let agent = ureq::builder()
///     .middleware(my_middleware)
///     .build();
///
/// let result: serde_json::Value =
///     agent.get("http://httpbin.org/headers").call()?.into_json()?;
///
/// assert_eq!(&result["headers"]["X-My-Header"], "value_42");
///
/// # Ok(()) }
/// # #[cfg(not(feature = "json"))]
/// # fn main() {}
/// ```
///
/// # State
///
/// To maintain state between middleware invocations, we need to do something more elaborate than
/// the simple `fn` and implement the `Middleware` trait directly.
///
/// ## Example with mutex lock
///
/// In the `examples` directory there is an additional example `count-bytes.rs` which uses
/// a mutex lock like shown below.
///
/// ```
/// # use ureq::{Request, Response, Middleware, MiddlewareNext, Error};
/// # use std::sync::{Arc, Mutex};
/// struct MyState {
///     // whatever is needed
/// }
///
/// struct MyMiddleware(Arc<Mutex<MyState>>);
///
/// impl Middleware for MyMiddleware {
///     fn handle(&self, request: Request, next: MiddlewareNext) -> Result<Response, Error> {
///         // These extra brackets ensures we release the Mutex lock before continuing the
///         // chain. There could also be scenarios where we want to maintain the lock through
///         // the invocation, which would block other requests from proceeding concurrently
///         // through the middleware.
///         {
///             let mut state = self.0.lock().unwrap();
///             // do stuff with state
///         }
///
///         // continue middleware chain
///         next.handle(request)
///     }
/// }
/// ```
///
/// ## Example with atomic
///
/// This example shows how we can increase a counter for each request going
/// through the agent.
///
/// ```
/// # fn main() -> Result<(), ureq::Error> {
/// # ureq::is_test(true);
/// use ureq::{Request, Response, Middleware, MiddlewareNext, Error};
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use std::sync::Arc;
///
/// // Middleware that stores a counter state. This example uses an AtomicU64
/// // since the middleware is potentially shared by multiple threads running
/// // requests at the same time.
/// struct MyCounter(Arc<AtomicU64>);
///
/// impl Middleware for MyCounter {
///     fn handle(&self, req: Request, next: MiddlewareNext) -> Result<Response, Error> {
///         // increase the counter for each invocation
///         self.0.fetch_add(1, Ordering::SeqCst);
///
///         // continue the middleware chain
///         next.handle(req)
///     }
/// }
///
/// let shared_counter = Arc::new(AtomicU64::new(0));
///
/// let agent = ureq::builder()
///     // Add our middleware
///     .middleware(MyCounter(shared_counter.clone()))
///     .build();
///
/// agent.get("http://httpbin.org/get").call()?;
/// agent.get("http://httpbin.org/get").call()?;
///
/// // Check we did indeed increase the counter twice.
/// assert_eq!(shared_counter.load(Ordering::SeqCst), 2);
///
/// # Ok(()) }
/// ```
pub trait Middleware: Send + Sync + 'static {
    /// Handle of the middleware logic.
    fn handle(&self, request: Request, next: MiddlewareNext) -> Result<Response, Error>;
}

/// Continuation of a [`Middleware`] chain.
pub struct MiddlewareNext<'a> {
    pub(crate) chain: &'a mut (dyn Iterator<Item = &'a dyn Middleware>),
    // Since request_fn consumes the Payload<'a>, we must have an FnOnce.
    //
    // It's possible to get rid of this Box if we make MiddlewareNext generic
    // over some type variable, i.e. MiddlewareNext<'a, R> where R: FnOnce...
    // however that would "leak" to Middleware::handle introducing a complicated
    // type signature that is totally irrelevant for someone implementing a middleware.
    //
    // So in the name of having a sane external API, we accept this Box.
    pub(crate) request_fn: Box<dyn FnOnce(Request) -> Result<Response, Error> + 'a>,
}

impl<'a> MiddlewareNext<'a> {
    /// Continue the middleware chain by providing (a possibly amended) [`Request`].
    pub fn handle(self, request: Request) -> Result<Response, Error> {
        if let Some(step) = self.chain.next() {
            step.handle(request, self)
        } else {
            (self.request_fn)(request)
        }
    }
}

impl<F> Middleware for F
where
    F: Fn(Request, MiddlewareNext) -> Result<Response, Error> + Send + Sync + 'static,
{
    fn handle(&self, request: Request, next: MiddlewareNext) -> Result<Response, Error> {
        (self)(request, next)
    }
}

#[cfg(feature = "digest-auth")]
pub mod digest {
    use super::*;
    use crate::{Request, Response};
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
    ///     ureq::DigestAuthMiddleware::new(arbitrary_username.into(), arbitrary_password.into());
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
        pub fn new(username: Cow<'static, str>, password: Cow<'static, str>) -> Self {
            Self { username, password }
        }

        fn construct_answer_to_challenge(
            &self,
            request: &Request,
            response: &Response,
        ) -> Option<String> {
            let challenge_string = response.header("www-authenticate")?;
            let mut challenge = WwwAuthenticateHeader::from_str(&challenge_string).ok()?;
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
}
