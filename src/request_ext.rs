use crate::config::ConfigBuilder;
use crate::typestate::HttpCrateScope;
use crate::{http, Agent, AsSendBody};

/// Extension trait for [`http::Request<impl AsSendBody>`].
///
/// Adds additional convenience methods to the `Request` that are not available
/// in the plain http API.
pub trait RequestExt<S>
where
    S: AsSendBody,
{
    /// Allows ureq-specific configuration for this request.
    ///
    /// This function will return a `ConfigBuilder` that can be used to set ureq-specific options.
    /// The resulting method can be used with `ureq::run` or with an existing agent.
    /// Configuration values set on the request takes precedence over configuration values set on the agent.
    ///
    /// # Examples
    /// ```
    /// use ureq::http;
    /// use ureq::RequestExt;
    ///
    /// // Acquire an `http::Request` somehow
    /// let request: http::Request<()> = http::Request::builder()
    ///             .method(http::Method::GET)
    ///             .uri("http://foo.bar")
    ///             .body(())
    ///             .unwrap();
    ///
    /// // Use the `configure()` method to set ureq-specific options
    /// // and call `build()` to get the request back.
    /// let request: http::Request<()> = request
    ///     .configure()
    ///     .https_only(false)
    ///     .build();
    /// ```
    fn configure(self) -> ConfigBuilder<HttpCrateScope<S>>;
}

impl<S: AsSendBody> RequestExt<S> for http::Request<S> {
    fn configure(self) -> ConfigBuilder<HttpCrateScope<S>> {
        let agent = Agent::new_with_defaults();
        agent.configure_request(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RequestLevelConfig;

    #[test]
    fn set_https_only_to_true_on_get_request() {
        // Create `http` crate request
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://foo.bar")
            .body(())
            .unwrap();

        // Configure with the trait
        let request = request.configure().https_only(true).build();

        let request_config = request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert_eq!(request_config.0.https_only(), true);
    }

    #[test]
    fn set_https_only_to_false_on_get_request() {
        // Create `http` crate request
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://foo.bar")
            .body(())
            .unwrap();

        // Configure with the trait
        let request = request.configure().https_only(false).build();

        let request_config = request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert_eq!(request_config.0.https_only(), false);
    }

    #[test]
    fn set_http_status_as_error_to_true_on_post_request() {
        // Create `http` crate request
        let request = http::Request::builder()
            .method(http::Method::POST)
            .uri("http://foo.bar")
            .body("Some body")
            .unwrap();

        // Configure with the trait
        let request = request.configure().http_status_as_error(true).build();

        let request_config = request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert_eq!(request_config.0.http_status_as_error(), true);
    }

    #[test]
    fn set_http_status_as_error_to_false_on_post_request() {
        // Create `http` crate request
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("http://foo.bar")
            .body("Some body")
            .unwrap();

        // Configure with the trait
        let request = request.configure().http_status_as_error(false).build();

        let request_config = request
            .extensions()
            .get::<RequestLevelConfig>()
            .cloned()
            .unwrap();

        assert_eq!(request_config.0.http_status_as_error(), false);
    }
}
