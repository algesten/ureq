use std::any::Any;

use crate::{Error, Request, Response};

/// Chained processing of request (and response).
///
/// # Adding headers
///
/// A common use case is to add headers to the outgoing request. Here an example of how.
///
/// ```
/// # #[cfg(feature = "json")]
/// # fn main() -> Result<(), ureq::Error> {
/// # use ureq::{Request, Response, MiddlewareRequestNext, RequestMiddleware, Error};
/// # ureq::is_test(true);
/// fn my_middleware(req: Request, next: MiddlewareRequestNext) -> Result<Request, Error> {
///     // set my bespoke header and continue the chain
///     next.handle((), req.set("X-My-Header", "value_42"))
/// }
///
/// let agent = ureq::builder()
///     .middleware(RequestMiddleware::new(my_middleware))
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
/// # use ureq::{Request, Response, Middleware, MiddlewareRequestNext, Error};
/// # use std::sync::{Arc, Mutex};
/// struct MyState {
///     // whatever is needed
/// }
///
/// struct MyMiddleware(Arc<Mutex<MyState>>);
///
/// impl Middleware for MyMiddleware {
///     fn handle_request(&self, request: Request, next: MiddlewareRequestNext) -> Result<Request, Error> {
///         let mut state = self.0.lock().unwrap();
///         // do stuff with state
///
///         // continue middleware chain
///         next.handle((), request)
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
/// use ureq::{Request, Response, Middleware, MiddlewareRequestNext, Error};
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use std::sync::Arc;
///
/// // Middleware that stores a counter state. This example uses an AtomicU64
/// // since the middleware is potentially shared by multiple threads running
/// // requests at the same time.
/// struct MyCounter(Arc<AtomicU64>);
///
/// impl Middleware for MyCounter {
///     fn handle_request(&self, req: Request, next: MiddlewareRequestNext) -> Result<Request, Error> {
///         // increase the counter for each invocation
///         self.0.fetch_add(1, Ordering::SeqCst);
///
///         // continue the middleware chain.
///         // first argument is request specific state for `handle_response`, but we don't need it.
///         next.handle((), req)
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
    /// Handle the Request side of requests made via [`Request::call_writer`].
    fn handle_request(
        &self,
        request: Request,
        next: MiddlewareRequestNext,
    ) -> Result<Request, Error> {
        next.handle((), request)
    }

    /// Handle the Response side of requests made via [`Request::call_writer`].
    ///
    /// `state` is the object passed as the first argument of [`MiddlewareRequestNext::handle`], wrapped in a `Box`.
    fn handle_response(
        &self,
        response: Response,
        state: Box<dyn Any + Send>,
        next: MiddlewareResponseNext,
    ) -> Result<Response, Error> {
        let _ = state;
        next.handle(response)
    }
}

/// Wraps a function, allowing it to implement [`Middleware::handle_request`].
///
/// If you want a middleware to only modify requests, and don't need to modify responses, you can
/// wrap a closure in this structure, which will forward [`Middleware::handle_request`] for you.
pub struct RequestMiddleware<F>(F);
impl<F> RequestMiddleware<F>
where
    F: Fn(Request, MiddlewareRequestNext) -> Result<Request, Error> + Send + Sync + 'static,
{
    /// Wraps a function.
    pub fn new(f: F) -> Self {
        Self(f)
    }
}
impl<F> Middleware for RequestMiddleware<F>
where
    F: Fn(Request, MiddlewareRequestNext) -> Result<Request, Error> + Send + Sync + 'static,
{
    fn handle_request(
        &self,
        request: Request,
        next: MiddlewareRequestNext,
    ) -> Result<Request, Error> {
        (self.0)(request, next)
    }
}
impl<F> From<F> for RequestMiddleware<F>
where
    F: Fn(Request, MiddlewareRequestNext) -> Result<Request, Error> + Send + Sync + 'static,
{
    fn from(f: F) -> Self {
        Self(f)
    }
}

/// Wraps a function, allowing it to implement [`Middleware::handle_response`].
///
/// If you want a middleware to only modify responses, and don't need to modify request, you can
/// wrap a closure in this structure, which will forward [`Middleware::handle_response`] for you.
pub struct ResponseMiddleware<F>(F);
impl<F> ResponseMiddleware<F>
where
    F: Fn(Response, MiddlewareResponseNext) -> Result<Response, Error> + Send + Sync + 'static,
{
    /// Wraps a function.
    pub fn new(f: F) -> Self {
        Self(f)
    }
}
impl<F> Middleware for ResponseMiddleware<F>
where
    F: Fn(Response, MiddlewareResponseNext) -> Result<Response, Error> + Send + Sync + 'static,
{
    fn handle_response(
        &self,
        response: Response,
        _: Box<dyn Any + Send>,
        next: MiddlewareResponseNext,
    ) -> Result<Response, Error> {
        (self.0)(response, next)
    }
}
impl<F> From<F> for ResponseMiddleware<F>
where
    F: Fn(Response, MiddlewareResponseNext) -> Result<Response, Error> + Send + Sync + 'static,
{
    fn from(f: F) -> Self {
        Self(f)
    }
}

/// Continuation of a [`Middleware`] chain for the [`Middleware::handle_request`] method.
pub struct MiddlewareRequestNext<'a> {
    pub(crate) current_slot: &'a mut Option<Box<dyn Any + Send>>,
    pub(crate) chain:
        &'a mut (dyn Iterator<Item = (&'a dyn Middleware, &'a mut Option<Box<dyn Any + Send>>)>),
}
impl<'a> MiddlewareRequestNext<'a> {
    /// Continue the middleware chain by providing (a possibly amended) [`Request`], as well as
    /// request-specific state that will be passed to the middleware in [`Middleware::handle_response`].
    pub fn handle<S: Any + Send + 'static>(
        self,
        state: S,
        request: Request,
    ) -> Result<Request, Error> {
        debug_assert!(self.current_slot.is_none());
        *self.current_slot = Some(Box::new(state));
        self.handle_no_data(request)
    }

    pub(crate) fn handle_no_data(mut self, request: Request) -> Result<Request, Error> {
        if let Some((step, data_slot)) = self.chain.next() {
            self.current_slot = data_slot;
            step.handle_request(request, self)
        } else {
            Ok(request)
        }
    }
}

/// Continuation of a [`Middleware`] chain for the [`Middleware::handle_response`] method.
pub struct MiddlewareResponseNext<'a> {
    pub(crate) chain:
        &'a mut (dyn Iterator<Item = (&'a dyn Middleware, &'a mut Option<Box<dyn Any + Send>>)>),
}
impl<'a> MiddlewareResponseNext<'a> {
    /// Continue the middleware chain by providing (a possibly amended) [`Response`].
    ///
    /// Panics
    /// ------
    ///
    /// If the middleware did not call [`MiddlewareRequestNext::handle`] in the corresponding
    /// [`Middleware::handle_request`] call (i.e. returned a new [`Request`] object),
    /// this will panic when called.
    pub fn handle(self, response: Response) -> Result<Response, Error> {
        if let Some((step, slot)) = self.chain.next() {
            let state = match slot.take() {
                Some(v) => v,
                None => panic!("Middleware handle_response tried to forward to next in chain, but did not forward to middleware in handle_request"),
            };
            step.handle_response(response, state, self)
        } else {
            Ok(response)
        }
    }
}

pub(crate) type MiddlewareData = Vec<Option<Box<dyn Any + Send>>>;

/// Executes the request side of all middlewares.
///
/// Returns the altered request and the request-unique state.
pub(crate) fn execute_request_middleware(
    middlewares: &[Box<dyn Middleware>],
    request: Request,
) -> Result<(Request, MiddlewareData), Error> {
    let mut middleware_states = vec![];
    middleware_states.resize_with(middlewares.len(), || None);
    let mut chain = middlewares
        .iter()
        .map(|mw| &**mw)
        .zip(middleware_states.iter_mut());
    let altered_req = MiddlewareRequestNext {
        current_slot: &mut None,
        chain: &mut chain,
    }
    .handle_no_data(request)?;
    Ok((altered_req, middleware_states))
}

/// Executes the response side of all middlewares.
pub(crate) fn execute_response_middleware(
    middlewares: &[Box<dyn Middleware>],
    mut middleware_states: MiddlewareData,
    response: Response,
) -> Result<Response, Error> {
    let mut chain = middlewares
        .iter()
        .map(|mw| &**mw)
        .zip(middleware_states.iter_mut());
    MiddlewareResponseNext { chain: &mut chain }.handle(response)
}

/*
impl<F> Middleware for F
where
    F: Fn(Request, MiddlewareNext) -> Result<Response, Error> + Send + Sync + 'static,
{
    fn handle(&self, request: Request, next: MiddlewareNext) -> Result<Response, Error> {
        (self)(request, next)
    }
}
*/
