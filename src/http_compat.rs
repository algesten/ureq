use std::io::Read;
use bytes::Bytes;
use http::Method;
use crate::{Agent, Error};

pub struct UreqBody(Bytes);

impl From<Bytes> for UreqBody {
    #[inline]
    fn from(bytes: Bytes) -> Self {
        Self(bytes)
    }
}

impl From<()> for UreqBody {
    fn from(_: ()) -> Self {
        Self(Bytes::new())
    }
}

impl From<Vec<u8>> for UreqBody {
    #[inline]
    fn from(buffer: Vec<u8>) -> Self {
        Self(buffer.into())
    }
}

impl From<&'static [u8]> for UreqBody {
    #[inline]
    fn from(slice: &'static [u8]) -> Self {
        Self(Bytes::from_static(slice))
    }
}

impl From<String> for UreqBody {
    #[inline]
    fn from(buffer: String) -> Self {
        buffer.into_bytes().into()
    }
}

impl From<&'static str> for UreqBody {
    #[inline]
    fn from(slice: &'static str) -> Self {
        slice.as_bytes().into()
    }
}

impl Agent {
    pub fn send_http<T: Into<UreqBody>>(&self, request: http::Request<T>) -> Result<http::Response<Bytes>, Error> {
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
        let bytes = Bytes::from(buffer);

        let http_response = builder.body(bytes)?;
        Ok(http_response)
    }
}
