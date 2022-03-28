use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use ureq::{Error, Middleware, MiddlewareRequestNext, MiddlewareResponseNext, Request, Response};

// Some state that could be shared with the main application.
#[derive(Debug, Default)]
struct CounterState {
    request_count: u64,
    total_bytes: u64,
}

// Middleware wrapper working off the shared state.
struct CounterMiddleware(Arc<Mutex<CounterState>>);

pub fn main() -> Result<(), Error> {
    // Shared state for counters.
    let shared_state = Arc::new(Mutex::new(CounterState::default()));

    let agent = ureq::builder()
        // Clone the state into the middleware
        .middleware(CounterMiddleware(shared_state.clone()))
        .build();

    agent.get("https://httpbin.org/bytes/123").call()?;
    agent.get("https://httpbin.org/bytes/123").call()?;

    {
        let state = shared_state.lock().unwrap();

        println!("State after requests:\n\n{:?}\n", state);

        assert_eq!(state.request_count, 2);
        assert_eq!(state.total_bytes, 246);
    }

    Ok(())
}

impl Middleware for CounterMiddleware {
    fn handle_request(
        &self,
        request: Request,
        next: MiddlewareRequestNext,
    ) -> Result<Request, Error> {
        let mut state = self.0.lock().unwrap();
        state.request_count += 1;

        // First argument is passed into handle_response, as a `Box<dyn Any + Send>`.
        // This example does not need it, so just pass in `()`
        next.handle((), request)
    }

    fn handle_response(
        &self,
        response: Response,
        req_state: Box<dyn Any + Send>,
        next: MiddlewareResponseNext,
    ) -> Result<Response, Error> {
        // State give in `handle`.
        let () = *req_state.downcast().unwrap();

        let response = next.handle(response)?;

        let mut state = self.0.lock().unwrap();

        let len = response
            .header("Content-Length")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap();

        state.total_bytes += len;

        Ok(response)
    }
}
