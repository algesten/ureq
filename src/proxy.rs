use std::convert::{TryFrom, TryInto};
use std::fmt;

use http::Uri;

use crate::util::{AuthorityExt, DebugUri};
use crate::Error;

/// Proxy protocol
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Proto {
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
}

/// Proxy server definition
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct Proxy {
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
    /// * `http`: HTTP
    /// * `socks4`: SOCKS4 (requires socks feature)
    /// * `socks4a`: SOCKS4A (requires socks feature)
    /// * `socks5` and `socks`: SOCKS5 (requires socks feature)
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

        Ok(Self {
            proto,
            uri,
            from_env,
        })
    }

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

    pub fn proto(&self) -> Proto {
        self.proto
    }

    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    pub fn host(&self) -> &str {
        self.uri
            .authority()
            .map(|a| a.host())
            .expect("constructor to ensure there is an authority")
    }

    pub fn port(&self) -> u16 {
        self.uri
            .authority()
            .and_then(|a| a.port_u16())
            .unwrap_or_else(|| self.proto.default_port())
    }

    pub fn username(&self) -> Option<&str> {
        self.uri.authority().and_then(|a| a.username())
    }

    pub fn password(&self) -> Option<&str> {
        self.uri.authority().and_then(|a| a.password())
    }

    pub fn is_from_env(&self) -> bool {
        self.from_env
    }

    //     pub(crate) fn connect<S: AsRef<str>>(&self, host: S, port: u16, user_agent: &str) -> String {
    //         let authorization = if self.use_authorization() {
    //             let creds = BASE64_STANDARD.encode(format!(
    //                 "{}:{}",
    //                 self.username.clone().unwrap_or_default(),
    //                 self.password.clone().unwrap_or_default()
    //             ));

    //             match self.proto {
    //                 Proto::HTTP => format!("Proxy-Authorization: basic {}\r\n", creds),
    //                 _ => String::new(),
    //             }
    //         } else {
    //             String::new()
    //         };

    //         format!(
    //             "CONNECT {}:{} HTTP/1.1\r\n\
    // Host: {}:{}\r\n\
    // User-Agent: {}\r\n\
    // Proxy-Connection: Keep-Alive\r\n\
    // {}\
    // \r\n",
    //             host.as_ref(),
    //             port,
    //             host.as_ref(),
    //             port,
    //             user_agent,
    //             authorization
    //         )
    //     }

    // pub(crate) fn verify_response(response: &Response) -> Result<(), Error> {
    //     match response.status() {
    //         200 => Ok(()),
    //         401 | 407 => Err(ErrorKind::ProxyUnauthorized.new()),
    //         _ => Err(ErrorKind::ProxyConnect.new()),
    //     }
    // }
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
            .field("proto", &self.proto)
            .field("uri", &DebugUri(&self.uri))
            .field("from_env", &self.from_env)
            .finish()
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
        assert_eq!(proxy.proto, Proto::Http);
    }

    #[test]
    fn parse_proxy_http_user_pass_server_port_trailing_slash() {
        let proxy = Proxy::new("http://user:p@ssw0rd@localhost:9999/").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.proto, Proto::Http);
    }

    #[test]
    fn parse_proxy_socks4_user_pass_server_port() {
        let proxy = Proxy::new("socks4://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.proto, Proto::Socks4);
    }

    #[test]
    fn parse_proxy_socks4a_user_pass_server_port() {
        let proxy = Proxy::new("socks4a://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.proto, Proto::Socks4A);
    }

    #[test]
    fn parse_proxy_socks_user_pass_server_port() {
        let proxy = Proxy::new("socks://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.proto, Proto::Socks5);
    }

    #[test]
    fn parse_proxy_socks5_user_pass_server_port() {
        let proxy = Proxy::new("socks5://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.proto, Proto::Socks5);
    }

    #[test]
    fn parse_proxy_user_pass_server_port() {
        let proxy = Proxy::new("user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.proto, Proto::Http);
    }

    #[test]
    fn parse_proxy_server_port() {
        let proxy = Proxy::new("localhost:9999").unwrap();
        assert_eq!(proxy.username(), None);
        assert_eq!(proxy.password(), None);
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.proto, Proto::Http);
    }

    #[test]
    fn parse_proxy_server() {
        let proxy = Proxy::new("localhost").unwrap();
        assert_eq!(proxy.username(), None);
        assert_eq!(proxy.password(), None);
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 80);
        assert_eq!(proxy.proto, Proto::Http);
    }
}
