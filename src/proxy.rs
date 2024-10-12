use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use hoot::parser::try_parse_response;
use std::convert::{TryFrom, TryInto};
use std::fmt;
use std::io::Write;
use std::sync::Arc;

use http::{StatusCode, Uri};

use crate::transport::{ConnectionDetails, Connector, Transport, TransportAdapter};
use crate::util::{AuthorityExt, DebugUri, SchemeExt, UriExt};
use crate::Error;

/// Proxy protocol
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub(crate) enum Proto {
    Http,
    Https,
    Socks4,
    Socks4A,
    Socks5,
}

impl Proto {
    pub fn default_port(&self) -> u16 {
        match self {
            Proto::Http => 80,
            Proto::Https => 443,
            Proto::Socks4 | Proto::Socks4A | Proto::Socks5 => 1080,
        }
    }

    pub fn is_socks(&self) -> bool {
        matches!(self, Self::Socks4 | Self::Socks4A | Self::Socks5)
    }

    pub(crate) fn is_connect(&self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }
}

/// Proxy server settings
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct Proxy {
    inner: Arc<ProxyInner>,
}

#[derive(Eq, Hash, PartialEq)]
struct ProxyInner {
    proto: Proto,
    uri: Uri,
    from_env: bool,
}

impl Proxy {
    /// Create a proxy from a uri.
    ///
    /// # Arguments:
    ///
    /// * `proxy` - a str of format `<protocol>://<user>:<password>@<host>:port` . All parts
    ///    except host are optional.
    ///
    /// ###  Protocols
    ///
    /// * `http`: HTTP CONNECT proxy
    /// * `https`: HTTPS CONNECT proxy (requires a TLS provider)
    /// * `socks4`: SOCKS4 (requires **socks-proxy** feature)
    /// * `socks4a`: SOCKS4A (requires **socks-proxy** feature)
    /// * `socks5` and `socks`: SOCKS5 (requires **socks-proxy** feature)
    ///
    /// # Examples proxy formats
    ///
    /// * `http://127.0.0.1:8080`
    /// * `socks5://john:smith@socks.google.com`
    /// * `john:smith@socks.google.com:8000`
    /// * `localhost`
    pub fn new(proxy: &str) -> Result<Self, Error> {
        Self::new_with_flag(proxy, false)
    }

    fn new_with_flag(proxy: &str, from_env: bool) -> Result<Self, Error> {
        let uri = proxy.parse::<Uri>().unwrap();

        // The uri must have an authority part (with the host), or
        // it is invalid.
        let _ = uri.authority().ok_or(Error::InvalidProxyUrl)?;

        // The default protocol is Proto::HTTP
        let scheme = uri.scheme_str().unwrap_or("http");
        let proto = scheme.try_into()?;

        let inner = ProxyInner {
            proto,
            uri,
            from_env,
        };

        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Read proxy settings from environment variables.
    ///
    /// The environment variable is expected to contain a proxy URI. The following
    /// environment variables are attempted:
    ///
    /// * `ALL_PROXY`
    /// * `HTTPS_PROXY`
    /// * `HTTP_PROXY`
    ///
    /// Returns `None` if no environment variable is set or the URI is invalid.
    pub fn try_from_env() -> Option<Self> {
        macro_rules! try_env {
            ($($env:literal),+) => {
                $(
                    if let Ok(env) = std::env::var($env) {
                        if let Ok(proxy) = Self::new_with_flag(&env, true) {
                            return Some(proxy);
                        }
                    }
                )+
            };
        }

        try_env!(
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy"
        );
        None
    }

    pub(crate) fn proto(&self) -> Proto {
        self.inner.proto
    }

    /// The proxy uri
    pub fn uri(&self) -> &Uri {
        &self.inner.uri
    }

    /// The host part of the proxy uri
    pub fn host(&self) -> &str {
        self.inner
            .uri
            .authority()
            .map(|a| a.host())
            .expect("constructor to ensure there is an authority")
    }

    /// The port of the proxy uri
    pub fn port(&self) -> u16 {
        self.inner
            .uri
            .authority()
            .and_then(|a| a.port_u16())
            .unwrap_or_else(|| self.inner.proto.default_port())
    }

    /// The username of the proxy uri
    pub fn username(&self) -> Option<&str> {
        self.inner.uri.authority().and_then(|a| a.username())
    }

    /// The password of the proxy uri
    pub fn password(&self) -> Option<&str> {
        self.inner.uri.authority().and_then(|a| a.password())
    }

    /// Whether this proxy setting was created manually or from
    /// environment variables.
    pub fn is_from_env(&self) -> bool {
        self.inner.from_env
    }
}

/// Connector for CONNECT proxy settings.
///
/// This operates on the previous chained transport typically a TcpConnector optionally
/// wrapped in TLS.
pub struct ConnectProxyConnector;

impl Connector for ConnectProxyConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        let Some(transport) = chained else {
            return Ok(None);
        };

        let is_connect_proxy = details.config.connect_proxy_uri().is_some();

        if is_connect_proxy {
            // unwrap is ok because connect_proxy_uri() above checks it.
            let proxy = details.config.proxy.as_ref().unwrap();

            let mut w = TransportAdapter::new(transport);

            let uri = &details.uri;
            uri.ensure_valid_url()?;

            // All these unwrap() are ok because ensure_valid_uri() above checks them.
            let host = uri.host().unwrap();
            let port = uri
                .port_u16()
                .unwrap_or(uri.scheme().unwrap().default_port().unwrap());

            write!(w, "CONNECT {}:{} HTTP/1.1\r\n", host, port)?;
            write!(w, "Host: {}:{}\r\n", host, port)?;
            write!(w, "User-Agent: {}\r\n", details.config.get_user_agent())?;
            write!(w, "Proxy-Connection: Keep-Alive\r\n")?;

            let use_creds = proxy.username().is_some() || proxy.password().is_some();

            if use_creds {
                let user = proxy.username().unwrap_or_default();
                let pass = proxy.password().unwrap_or_default();
                let creds = BASE64_STANDARD.encode(format!("{}:{}", user, pass));
                write!(w, "Proxy-Authorization: basic {}\r\n", creds)?;
            }

            write!(w, "\r\n")?;
            w.flush()?;

            let mut transport = w.into_inner();

            let response = loop {
                let made_progress = transport.await_input(details.timeout)?;
                let buffers = transport.buffers();
                let input = buffers.input();
                let Some((used_input, response)) = try_parse_response::<20>(input)? else {
                    if !made_progress {
                        let reason = "proxy server did not respond".to_string();
                        return Err(Error::ConnectProxyFailed(reason));
                    }
                    continue;
                };
                buffers.input_consume(used_input);
                break response;
            };

            match response.status() {
                StatusCode::OK => {
                    trace!("CONNECT proxy connected");
                }
                x => {
                    let reason = format!("proxy server responded {}/{}", x.as_u16(), x.as_str());
                    return Err(Error::ConnectProxyFailed(reason));
                }
            }

            Ok(Some(transport))
        } else {
            Ok(Some(transport))
        }
    }
}

impl TryFrom<&str> for Proto {
    type Error = Error;

    fn try_from(scheme: &str) -> Result<Self, Self::Error> {
        match scheme.to_ascii_lowercase().as_str() {
            "http" => Ok(Proto::Http),
            "https" => Ok(Proto::Https),
            "socks4" => Ok(Proto::Socks4),
            "socks4a" => Ok(Proto::Socks4A),
            "socks" => Ok(Proto::Socks5),
            "socks5" => Ok(Proto::Socks5),
            _ => Err(Error::InvalidProxyUrl),
        }
    }
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Proxy")
            .field("proto", &self.inner.proto)
            .field("uri", &DebugUri(&self.inner.uri))
            .field("from_env", &self.inner.from_env)
            .finish()
    }
}

impl fmt::Display for Proto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Proto::Http => write!(f, "HTTP"),
            Proto::Https => write!(f, "HTTPS"),
            Proto::Socks4 => write!(f, "SOCKS4"),
            Proto::Socks4A => write!(f, "SOCKS4a"),
            Proto::Socks5 => write!(f, "SOCKS5"),
        }
    }
}

impl fmt::Debug for ConnectProxyConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyConnector").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_fakeproto() {
        assert!(Proxy::new("fakeproto://localhost").is_err());
    }

    #[test]
    fn parse_proxy_http_user_pass_server_port() {
        let proxy = Proxy::new("http://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, Proto::Http);
    }

    #[test]
    fn parse_proxy_http_user_pass_server_port_trailing_slash() {
        let proxy = Proxy::new("http://user:p@ssw0rd@localhost:9999/").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, Proto::Http);
    }

    #[test]
    fn parse_proxy_socks4_user_pass_server_port() {
        let proxy = Proxy::new("socks4://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, Proto::Socks4);
    }

    #[test]
    fn parse_proxy_socks4a_user_pass_server_port() {
        let proxy = Proxy::new("socks4a://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, Proto::Socks4A);
    }

    #[test]
    fn parse_proxy_socks_user_pass_server_port() {
        let proxy = Proxy::new("socks://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, Proto::Socks5);
    }

    #[test]
    fn parse_proxy_socks5_user_pass_server_port() {
        let proxy = Proxy::new("socks5://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, Proto::Socks5);
    }

    #[test]
    fn parse_proxy_user_pass_server_port() {
        let proxy = Proxy::new("user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, Proto::Http);
    }

    #[test]
    fn parse_proxy_server_port() {
        let proxy = Proxy::new("localhost:9999").unwrap();
        assert_eq!(proxy.username(), None);
        assert_eq!(proxy.password(), None);
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, Proto::Http);
    }

    #[test]
    fn parse_proxy_server() {
        let proxy = Proxy::new("localhost").unwrap();
        assert_eq!(proxy.username(), None);
        assert_eq!(proxy.password(), None);
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 80);
        assert_eq!(proxy.inner.proto, Proto::Http);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use assert_no_alloc::*;

    #[test]
    fn proxy_clone_does_not_allocate() {
        let c = Proxy::new("socks://1.2.3.4").unwrap();
        assert_no_alloc(|| c.clone());
    }
}
