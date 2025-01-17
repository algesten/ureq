use http::Uri;

use crate::body::Body;
use crate::http;

#[derive(Debug, Clone)]
pub(crate) struct ResponseUri(pub http::Uri);

#[derive(Debug, Clone)]
pub(crate) struct RedirectHistory(pub Vec<Uri>);

/// Extension trait for [`http::Response<Body>`].
///
/// Adds additional convenience methods to the `Response` that are not available
/// in the plain http API.
pub trait ResponseExt {
    /// The Uri that ultimately this Response is about.
    ///
    /// This can differ from the request uri when we have followed redirects.
    ///
    /// ```
    /// use ureq::ResponseExt;
    ///
    /// let res = ureq::get("https://httpbin.org/redirect-to?url=%2Fget")
    ///     .call().unwrap();
    ///
    /// assert_eq!(res.get_uri(), "https://httpbin.org/get");
    /// ```
    fn get_uri(&self) -> &Uri;

    /// The full history of uris, including the request and final uri.
    ///
    /// Returns `None` when [`Config::save_redirect_history`][crate::config::Config::save_redirect_history]
    /// is `false`.
    ///
    ///
    /// ```
    /// # use ureq::http::Uri;
    /// use ureq::ResponseExt;
    ///
    /// let uri1: Uri = "https://httpbin.org/redirect-to?url=%2Fget".parse().unwrap();
    /// let uri2: Uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
    ///
    /// let res = ureq::get(&uri1)
    ///     .config()
    ///     .save_redirect_history(true)
    ///     .build()
    ///     .call().unwrap();
    ///
    /// let history = res.get_redirect_history().unwrap();
    ///
    /// assert_eq!(history, &[uri1, uri2]);
    /// ```
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
