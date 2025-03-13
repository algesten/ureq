//! Chained interception to modify the request or response.

use std::fmt;
use std::sync::Arc;

use crate::http;
use crate::run::run;
use crate::{Agent, Body, Error, SendBody};

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
/// ```
/// use ureq::{Body, SendBody};
/// use ureq::middleware::MiddlewareNext;
/// use ureq::http::{Request, Response};
///
/// fn my_middleware(req: Request<SendBody>, next: MiddlewareNext)
///     -> Result<Response<Body>, ureq::Error> {
///
///     // do middleware things to request
///
///     // continue the middleware chain
///     let res = next.handle(req)?;
///
///     // do middleware things to response
///
///     Ok(res)
/// }
/// ```
///
/// # Adding headers
///
/// A common use case is to add headers to the outgoing request. Here an example of how.
///
/// ```no_run
/// use ureq::{Body, SendBody, Agent, config::Config};
/// use ureq::middleware::MiddlewareNext;
/// use ureq::http::{Request, Response, header::HeaderValue};
///
/// # #[cfg(feature = "json")]
/// # {
/// fn my_middleware(mut req: Request<SendBody>, next: MiddlewareNext)
///     -> Result<Response<Body>, ureq::Error> {
///
///     req.headers_mut().insert("X-My-Header", HeaderValue::from_static("value_42"));
///
///     // set my bespoke header and continue the chain
///     next.handle(req)
/// }
///
/// let mut config = Config::builder()
///     .middleware(my_middleware)
///     .build();
///
/// let agent: Agent = config.into();
///
/// let result: serde_json::Value =
///     agent.get("http://httpbin.org/headers").call()?.body_mut().read_json()?;
///
/// assert_eq!(&result["headers"]["X-My-Header"], "value_42");
/// # } Ok::<_, ureq::Error>(())
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
/// use std::sync::{Arc, Mutex};
///
/// use ureq::{Body, SendBody};
/// use ureq::middleware::{Middleware, MiddlewareNext};
/// use ureq::http::{Request, Response};
///
/// struct MyState {
///     // whatever is needed
/// }
///
/// struct MyMiddleware(Arc<Mutex<MyState>>);
///
/// impl Middleware for MyMiddleware {
///     fn handle(&self, request: Request<SendBody>, next: MiddlewareNext)
///         -> Result<Response<Body>, ureq::Error> {
///
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
/// use ureq::{Body, SendBody, Agent, config::Config};
/// use ureq::middleware::{Middleware, MiddlewareNext};
/// use ureq::http::{Request, Response};
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use std::sync::Arc;
///
/// // Middleware that stores a counter state. This example uses an AtomicU64
/// // since the middleware is potentially shared by multiple threads running
/// // requests at the same time.
/// struct MyCounter(Arc<AtomicU64>);
///
/// impl Middleware for MyCounter {
///     fn handle(&self, req: Request<SendBody>, next: MiddlewareNext)
///         -> Result<Response<Body>, ureq::Error> {
///
///         // increase the counter for each invocation
///         self.0.fetch_add(1, Ordering::Relaxed);
///
///         // continue the middleware chain
///         next.handle(req)
///     }
/// }
///
/// let shared_counter = Arc::new(AtomicU64::new(0));
///
/// let mut config = Config::builder()
///     .middleware(MyCounter(shared_counter.clone()))
///     .build();
///
/// let agent: Agent = config.into();
///
/// agent.get("http://httpbin.org/get").call()?;
/// agent.get("http://httpbin.org/get").call()?;
///
/// // Check we did indeed increase the counter twice.
/// assert_eq!(shared_counter.load(Ordering::Relaxed), 2);
///
/// # Ok::<_, ureq::Error>(())
/// ```
pub trait Middleware: Send + Sync + 'static {
    /// Handle of the middleware logic.
    fn handle(
        &self,
        request: http::Request<SendBody>,
        next: MiddlewareNext,
    ) -> Result<http::Response<Body>, Error>;
}

#[derive(Clone, Default)]
pub(crate) struct MiddlewareChain {
    chain: Arc<Vec<Box<dyn Middleware>>>,
}

impl MiddlewareChain {
    pub(crate) fn add(&mut self, mw: impl Middleware) {
        let Some(chain) = Arc::get_mut(&mut self.chain) else {
            panic!("Can't add to a MiddlewareChain that is already cloned")
        };

        chain.push(Box::new(mw));
    }
}

/// Continuation of a [`Middleware`] chain.
pub struct MiddlewareNext<'a> {
    agent: &'a Agent,
    index: usize,
}

impl<'a> MiddlewareNext<'a> {
    pub(crate) fn new(agent: &'a Agent) -> Self {
        MiddlewareNext { agent, index: 0 }
    }

    /// Continue the middleware chain.
    ///
    /// The middleware must call this in order to run the request. Not calling
    /// it is a valid choice for not wanting the request to execute.
    pub fn handle(
        mut self,
        request: http::Request<SendBody>,
    ) -> Result<http::Response<Body>, Error> {
        if let Some(mw) = self.agent.config().middleware.chain.get(self.index) {
            // This middleware exists, run it.
            self.index += 1;
            mw.handle(request, self)
        } else {
            // When chain is over, call the main run().
            let (parts, body) = request.into_parts();
            let request = http::Request::from_parts(parts, ());
            run(self.agent, request, body)
        }
    }
}

impl<F> Middleware for F
where
    F: Fn(http::Request<SendBody>, MiddlewareNext) -> Result<http::Response<Body>, Error>
        + Send
        + Sync
        + 'static,
{
    fn handle(
        &self,
        request: http::Request<SendBody>,
        next: MiddlewareNext,
    ) -> Result<http::Response<Body>, Error> {
        (self)(request, next)
    }
}

impl fmt::Debug for MiddlewareChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MiddlewareChain")
            .field("len", &self.chain.len())
            .finish()
    }
}
