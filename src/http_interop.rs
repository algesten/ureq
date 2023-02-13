use std::{
    io::{Cursor, Read},
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use crate::{header::HeaderLine, response::ResponseStatusIndex, Request, Response};

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

impl From<Response> for http::Response<Box<dyn Read + Send + Sync + 'static>> {
    fn from(value: Response) -> Self {
        create_builder(&value).body(value.into_reader()).unwrap()
    }
}

impl From<Response> for http::Response<String> {
    fn from(value: Response) -> Self {
        create_builder(&value)
            .body(value.into_string().unwrap())
            .unwrap()
    }
}

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
}
