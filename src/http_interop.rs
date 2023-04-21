use std::{
    io::{Cursor, Read},
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use crate::{header::HeaderLine, response::ResponseStatusIndex, Request, Response};

/// Converts an [http::Response] into a [Response](crate::Response).
///
/// As an [http::Response] does not contain a URL, `"https://example.com/"` is
/// used as a placeholder. Additionally, if the response has a header which
/// cannot be converted into a valid [Header](crate::Header), it will be skipped
/// rather than having the conversion fail. The remote address property will
/// also always be `127.0.0.1:80` for similar reasons to the URL.
///
/// Requires feature `ureq = { version = "*", features = ["http"] }`
/// ```
/// # fn main() -> Result<(), http::Error> {
/// # ureq::is_test(true);
/// let http_response = http::Response::builder().status(200).body("<response>")?;
/// let response: ureq::Response = http_response.into();
/// # Ok(())
/// # }
/// ```
impl<T: AsRef<[u8]> + Send + Sync + 'static> From<http::Response<T>> for Response {
    fn from(value: http::Response<T>) -> Self {
        let version_str = format!("{:?}", value.version());
        let status_line = format!("{} {}", version_str, value.status());
        let status_num = u16::from(value.status());
        Response {
            url: "https://example.com/".parse().unwrap(),
            status_line,
            index: ResponseStatusIndex {
                http_version: version_str.len(),
                response_code: version_str.len() + status_num.to_string().len(),
            },
            status: status_num,
            headers: value
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    let mut raw_header: Vec<u8> = name.to_string().into_bytes();
                    raw_header.extend([0x3a, 0x20]); // ": "
                    raw_header.extend(value.as_bytes());

                    HeaderLine::from(raw_header).into_header().ok()
                })
                .collect::<Vec<_>>(),
            reader: Box::new(Cursor::new(value.into_body())),
            remote_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 80),
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            history: vec![],
        }
    }
}

fn create_builder(response: &Response) -> http::response::Builder {
    let http_version = match response.http_version() {
        "HTTP/0.9" => http::Version::HTTP_09,
        "HTTP/1.0" => http::Version::HTTP_10,
        "HTTP/1.1" => http::Version::HTTP_11,
        "HTTP/2.0" => http::Version::HTTP_2,
        "HTTP/3.0" => http::Version::HTTP_3,
        _ => unreachable!(),
    };

    let response_builder = response
        .headers
        .iter()
        .filter_map(|header| {
            header
                .value()
                .map(|safe_value| (header.name().to_owned(), safe_value.to_owned()))
        })
        .fold(http::Response::builder(), |builder, header| {
            builder.header(header.0, header.1)
        })
        .status(response.status())
        .version(http_version);

    response_builder
}

/// Converts a [Response](crate::Response) into an [http::Response], where the
/// body is a reader containing the body of the response.
///
/// Due to slight differences in how headers are handled, this means if a header
/// from a [Response](crate::Response) is not valid UTF-8, it will not be
/// included in the resulting [http::Response].
///
/// Requires feature `ureq = { version = "*", features = ["http"] }`
/// ```
/// # fn main() -> Result<(), ureq::Error> {
/// # ureq::is_test(true);
/// use std::io::Read;
/// let response = ureq::get("http://example.com").call()?;
/// let http_response: http::Response<Box<dyn Read + Send + Sync + 'static>> = response.into();
/// # Ok(())
/// # }
/// ```
impl From<Response> for http::Response<Box<dyn Read + Send + Sync + 'static>> {
    fn from(value: Response) -> Self {
        create_builder(&value).body(value.into_reader()).unwrap()
    }
}

/// Converts a [Response](crate::Response) into an [http::Response], where the
/// body is a String.
///
/// Due to slight differences in how headers are handled, this means if a header
/// from a [Response](crate::Response) is not valid UTF-8, it will not be
/// included in the resulting [http::Response].
///
/// Requires feature `ureq = { version = "*", features = ["http"] }`
/// ```
/// # fn main() -> Result<(), ureq::Error> {
/// # ureq::is_test(true);
/// let response = ureq::get("http://example.com").call()?;
/// let http_response: http::Response<String> = response.into();
/// # Ok(())
/// # }
/// ```
impl From<Response> for http::Response<String> {
    fn from(value: Response) -> Self {
        create_builder(&value)
            .body(value.into_string().unwrap())
            .unwrap()
    }
}

/// Converts an [http] [Builder](http::request::Builder) into a [Request](crate::Request)
///
/// This will safely handle cases where a builder is not fully "complete" to
/// prevent the conversion from failing. Should the requests' method or URI not
/// be correctly set, the request will default to being a GET request to
/// `"https://example.com"`. Additionally, any non-UTF8 headers will be skipped.
///
/// Requires feature `ureq = { version = "*", features = ["http"] }`
/// ```
/// # fn main() -> Result<(), ureq::Error> {
/// # ureq::is_test(true);
/// let http_request_builder = http::Request::builder().method("GET").uri("http://example.com");
/// let request: ureq::Request = http_request_builder.into();
/// request.call()?;
/// # Ok(())
/// # }
/// ```
impl From<http::request::Builder> for Request {
    fn from(value: http::request::Builder) -> Self {
        let mut new_request = crate::agent().request(
            value.method_ref().map_or("GET", |m| m.as_str()),
            &value
                .uri_ref()
                .map_or("https://example.com".to_string(), |u| u.to_string()),
        );

        if let Some(headers) = value.headers_ref() {
            new_request = headers
                .iter()
                .filter_map(|header| {
                    header
                        .1
                        .to_str()
                        .ok()
                        .map(|str_value| (header.0.as_str(), str_value))
                })
                .fold(new_request, |request, header| {
                    request.set(header.0, header.1)
                });
        }

        new_request
    }
}

/// Converts a [Request](crate::Request) into an [http] [Builder](http::request::Builder).
///
/// This will only convert valid UTF-8 header values into headers on the
/// resulting builder. The method and URI are preserved. The HTTP version will
/// always be set to `HTTP/1.1`.
///
/// Requires feature `ureq = { version = "*", features = ["http"] }`
/// ```
/// # fn main() -> Result<(), http::Error> {
/// # ureq::is_test(true);
/// let request = ureq::get("https://my-website.com");
/// let http_request_builder: http::request::Builder = request.into();
///
/// http_request_builder.body(())?;
/// # Ok(())
/// # }
/// ```
impl From<Request> for http::request::Builder {
    fn from(value: Request) -> Self {
        value
            .headers
            .iter()
            .filter_map(|header| {
                header
                    .value()
                    .map(|safe_value| (header.name().to_owned(), safe_value.to_owned()))
            })
            .fold(http::Request::builder(), |builder, header| {
                builder.header(header.0, header.1)
            })
            .method(value.method())
            .version(http::Version::HTTP_11)
            .uri(value.url())
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn convert_http_response() {
        use http::{Response, StatusCode, Version};

        let http_response_body = (0..10240).into_iter().map(|_| 0xaa).collect::<Vec<u8>>();
        let http_response = Response::builder()
            .version(Version::HTTP_2)
            .header("Custom-Header", "custom value")
            .header("Content-Type", "application/octet-stream")
            .status(StatusCode::IM_A_TEAPOT)
            .body(http_response_body.clone())
            .unwrap();

        let response: super::Response = http_response.into();
        assert_eq!(response.get_url(), "https://example.com/");
        assert_eq!(response.http_version(), "HTTP/2.0");
        assert_eq!(response.status(), u16::from(StatusCode::IM_A_TEAPOT));
        assert_eq!(response.status_text(), "I'm a teapot");
        assert_eq!(response.remote_addr().to_string().as_str(), "127.0.0.1:80");
        assert_eq!(response.header("Custom-Header"), Some("custom value"));
        assert_eq!(response.content_type(), "application/octet-stream");

        let mut body_buf: Vec<u8> = vec![];
        response.into_reader().read_to_end(&mut body_buf).unwrap();
        assert_eq!(body_buf, http_response_body);
    }

    #[test]
    fn convert_http_response_string() {
        use http::{Response, StatusCode, Version};

        let http_response_body = "Some body string".to_string();
        let http_response = Response::builder()
            .version(Version::HTTP_11)
            .status(StatusCode::OK)
            .body(http_response_body.clone())
            .unwrap();

        let response: super::Response = http_response.into();
        assert_eq!(response.get_url(), "https://example.com/");
        assert_eq!(response.content_type(), "text/plain");
        assert_eq!(response.into_string().unwrap(), http_response_body);
    }

    #[test]
    fn convert_http_response_bad_header() {
        use http::{Response, StatusCode, Version};

        let http_response = Response::builder()
            .version(Version::HTTP_11)
            .status(StatusCode::OK)
            .header("Some-Invalid-Header", vec![0xde, 0xad, 0xbe, 0xef])
            .header("Some-Valid-Header", vec![0x48, 0x45, 0x4c, 0x4c, 0x4f])
            .body(vec![])
            .unwrap();

        let response: super::Response = http_response.into();
        assert_eq!(response.header("Some-Invalid-Header"), None);
        assert_eq!(response.header("Some-Valid-Header"), Some("HELLO"));
    }

    #[test]
    fn convert_to_http_response_string() {
        use http::Response;

        let mut response = super::Response::new(418, "I'm a teapot", "some body text").unwrap();
        response.headers.push(
            super::HeaderLine::from("Content-Type: text/plain".as_bytes().to_vec())
                .into_header()
                .unwrap(),
        );
        let http_response: Response<String> = response.into();

        assert_eq!(http_response.body(), "some body text");
        assert_eq!(http_response.status().as_u16(), 418);
        assert_eq!(
            http_response.status().canonical_reason(),
            Some("I'm a teapot")
        );
        assert_eq!(
            http_response
                .headers()
                .get("content-type")
                .map(|f| f.to_str().unwrap()),
            Some("text/plain")
        );
    }

    #[test]
    fn convert_to_http_response_bytes() {
        use http::Response;
        use std::io::{Cursor, Read};

        let mut response = super::Response::new(200, "OK", "tbr").unwrap();
        response.reader = Box::new(Cursor::new(vec![0xde, 0xad, 0xbe, 0xef]));
        let http_response: Response<Box<dyn Read + Send + Sync + 'static>> = response.into();

        let mut buf = vec![];
        http_response.into_body().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn convert_http_request_builder() {
        use http::Request;

        let http_request = Request::builder()
            .method("PUT")
            .header("Some-Key", "some value")
            .uri("https://google.com/?some=query");
        let request: super::Request = http_request.into();

        assert_eq!(request.header("some-key"), Some("some value"));
        assert_eq!(request.method(), "PUT");
        assert_eq!(request.url(), "https://google.com/?some=query");
    }

    #[test]
    fn convert_to_http_request_builder() {
        use http::request::Builder;

        let request = crate::agent()
            .head("http://some-website.com")
            .set("Some-Key", "some value");
        let http_request_builder: Builder = request.into();
        let http_request = http_request_builder.body(()).unwrap();

        assert_eq!(
            http_request
                .headers()
                .get("some-key")
                .map(|v| v.to_str().unwrap()),
            Some("some value")
        );
        assert_eq!(http_request.uri(), "http://some-website.com");
        assert_eq!(http_request.version(), http::Version::HTTP_11);
    }
}
