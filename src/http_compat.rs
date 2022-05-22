use std::io::Read;
use http::Method;
use crate::{Agent, Error};

pub struct UreqBody(Vec<u8>);

impl From<()> for UreqBody {
    fn from(_: ()) -> Self {
        Self(Vec::new())
    }
}

impl From<Vec<u8>> for UreqBody {
    #[inline]
    fn from(buffer: Vec<u8>) -> Self {
        Self(buffer)
    }
}

impl From<&'static [u8]> for UreqBody {
    #[inline]
    fn from(slice: &'static [u8]) -> Self {
        Self(Vec::from(slice))
    }
}

impl From<String> for UreqBody {
    #[inline]
    fn from(buffer: String) -> Self {
        Self(buffer.into_bytes())
    }
}

impl From<&'static str> for UreqBody {
    #[inline]
    fn from(slice: &'static str) -> Self {
        slice.as_bytes().into()
    }
}

impl Agent {
    /// Send requests using the `http::Request<T>` type.
    ///
    /// This supports any body type `T` for which there exists a `UreqBody::from` impl:
    ///
    /// - `impl From<()> for UreqBody`
    /// - `impl From<Vec<u8>> for UreqBody`
    /// - `impl From<&'static [u8]> for UreqBody`
    /// - `impl From<String> for UreqBody`
    /// - `impl From<&'static str> for UreqBody`
    ///
    /// # Example
    ///
    /// ```
    /// # fn example(agent: ureq::Agent) {
    /// let request = http::Request::builder()
    ///     .method(http::Method::GET)
    ///     .uri(http::Uri::from_static("http://example.com"))
    ///     .body("Hello, world!")
    ///     .unwrap();
    /// let response: http::Response<Vec<u8>> = agent.send_http(request).unwrap();
    /// # }
    /// ```
    pub fn send_http<T: Into<UreqBody>>(&self, request: http::Request<T>) -> Result<http::Response<Vec<u8>>, Error> {
        // Convert the http::Request to ureq::Request and execute it
        let (parts, body) = request.map(T::into).into_parts();
        let method = parts.method.as_str();
        let url = parts.uri.to_string();
        let response = self.request(method, &url).send(body.0)?;

        // Construct the http::Response from the ureq::Response
        let mut builder = http::Response::builder();
        let status = http::StatusCode::from_u16(response.status())?;
        builder = builder.status(status);

        for header_key in response.headers_names() {
            // Safety: We know this header exists because we got this key from the response
            let header_value = response.header(&header_key).unwrap();
            builder = builder.header(header_key, header_value);
        }

        // We need to read the whole body now, otherwise the socket will be dropped with the ureq::Response
        let body_len = response.header("Content-Length")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let mut buffer = Vec::with_capacity(body_len);
        response.into_reader().read_to_end(&mut buffer)?;

        let http_response = builder.body(buffer)?;
        Ok(http_response)
    }
}
