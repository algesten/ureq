use http::Uri;

use crate::body::Body;
use crate::http;

#[derive(Debug, Clone)]
pub(crate) struct ResponseUri(pub http::Uri);

#[derive(Debug, Clone)]
pub(crate) struct RedirectHistory(pub Vec<Uri>);

/// Extension trait for `http::Response<Body>` objects
///
/// Allows the user to access the `Uri` in http::Response
pub trait ResponseExt {
    /// The Uri we ended up at. This can differ from the request uri when we have followed redirects.
    fn get_uri(&self) -> &Uri;

    /// The full history of uris, including the request and final uri.
    ///
    /// Returns None when `save_redirect_history` is false.
    fn get_redirect_history(&self) -> Option<&[Uri]>;
}

impl ResponseExt for http::Response<Body> {
    fn get_uri(&self) -> &Uri {
        &self
            .extensions()
            .get::<ResponseUri>()
            .expect("uri to have been set")
            .0
    }

    fn get_redirect_history(&self) -> Option<&[Uri]> {
        self.extensions()
            .get::<RedirectHistory>()
            .map(|r| r.0.as_ref())
    }
}
