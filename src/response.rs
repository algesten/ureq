use http::Uri;

use crate::body::Body;
use crate::http;

#[derive(Debug, Clone)]
pub(crate) struct ResponseUri(pub http::Uri);

/// Extension trait for `http::Response<Body>` objects
///
/// Allows the user to access the `Uri` in http::Response
pub trait ResponseExt {
    /// The Uri we ended up at. This can differ from the request uri when we have followed redirects.
    fn get_uri(&self) -> &Uri;
}

impl ResponseExt for http::Response<Body> {
    fn get_uri(&self) -> &Uri {
        &self
            .extensions()
            .get::<ResponseUri>()
            .expect("uri to have been set")
            .0
    }
}
