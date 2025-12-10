use std::convert::{TryFrom, TryInto};
use std::fmt;
use std::sync::Arc;
use ureq_proto::http::uri::{PathAndQuery, Scheme};

use http::Uri;

use crate::http;
use crate::util::{AuthorityExt, DebugUri};
use crate::Error;

#[cfg(all(windows, feature = "win-system-proxy"))]
const REGISTRY_PATH: &str = r#"Software\Microsoft\Windows\CurrentVersion\Internet Settings"#;

/// Proxy protocol
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ProxyProtocol {
    /// CONNECT proxy over HTTP
    Http,
    /// CONNECT proxy over HTTPS
    Https,
    /// A SOCKS4 proxy
    Socks4,
    /// A SOCKS4a proxy (proxy can resolve domain name)
    Socks4A,
    /// SOCKS5 proxy
    Socks5,
    /// SOCKS5h proxy (proxy can resolve domain name)
    Socks5h,
}

impl ProxyProtocol {
    pub(crate) fn default_port(&self) -> u16 {
        match self {
            ProxyProtocol::Http => 80,
            ProxyProtocol::Https => 443,
            ProxyProtocol::Socks4
            | ProxyProtocol::Socks4A
            | ProxyProtocol::Socks5
            | ProxyProtocol::Socks5h => 1080,
        }
    }

    pub(crate) fn is_socks(&self) -> bool {
        matches!(
            self,
            Self::Socks4 | Self::Socks4A | Self::Socks5 | Self::Socks5h
        )
    }

    pub(crate) fn is_connect(&self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }

    fn default_resolve_target(&self) -> bool {
        match self {
            ProxyProtocol::Http => false,
            ProxyProtocol::Https => false,
            ProxyProtocol::Socks4 => true, // we must locally resolve before using proxy
            ProxyProtocol::Socks4A => false,
            ProxyProtocol::Socks5 => true, // we must locally resolve before using proxy
            ProxyProtocol::Socks5h => false,
        }
    }
}

/// Proxy server settings
///
/// This struct represents a proxy server configuration that can be used to route HTTP/HTTPS
/// requests through a proxy server. It supports various proxy protocols including HTTP CONNECT,
/// HTTPS CONNECT, SOCKS4, SOCKS4A, and SOCKS5.
///
/// # Protocol Support
///
/// * `HTTP`: HTTP CONNECT proxy
/// * `HTTPS`: HTTPS CONNECT proxy (requires a TLS provider)
/// * `SOCKS4`: SOCKS4 proxy (requires **socks-proxy** feature)
/// * `SOCKS4A`: SOCKS4A proxy (requires **socks-proxy** feature)
/// * `SOCKS5`: SOCKS5 proxy (requires **socks-proxy** feature)
///
/// # DNS Resolution
///
/// The `resolve_target` setting controls where DNS resolution happens:
///
/// * When `true`: DNS resolution happens locally before connecting to the proxy.
///   The resolved IP address is sent to the proxy.
/// * When `false`: The hostname is sent to the proxy, which performs DNS resolution.
///
/// Default behavior:
/// * For SOCKS4: `true` (local resolution required)
/// * For all other protocols: `false` (proxy performs resolution)
///
/// # Examples
///
/// ```rust
/// use ureq::{Proxy, ProxyProtocol};
///
/// // Create a proxy from a URI string
/// let proxy = Proxy::new("http://localhost:8080").unwrap();
///
/// // Create a proxy using the builder pattern
/// let proxy = Proxy::builder(ProxyProtocol::Socks5)
///     .host("proxy.example.com")
///     .port(1080)
///     .username("user")
///     .password("pass")
///     .resolve_target(true)  // Force local DNS resolution
///     .build()
///     .unwrap();
///
/// // Read proxy settings from environment variables
/// if let Some(proxy) = Proxy::try_from_env() {
///     // Use proxy from environment
/// }
/// ```
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct Proxy {
    inner: Arc<ProxyInner>,
}

#[derive(Eq, Hash, PartialEq)]
struct ProxyInner {
    proto: ProxyProtocol,
    uri: Uri,
    from_env: bool,
    resolve_target: bool,
    no_proxy: Option<NoProxy>,
}

impl Proxy {
    /// Create a proxy from a uri.
    ///
    /// # Arguments:
    ///
    /// * `proxy` - a str of format `<protocol>://<user>:<password>@<host>:port` . All parts
    ///   except host are optional.
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
        Self::new_with_flag(proxy, None, false, None)
    }

    /// Creates a proxy config using a builder.
    pub fn builder(p: ProxyProtocol) -> ProxyBuilder {
        ProxyBuilder {
            protocol: p,
            host: None,
            port: None,
            username: None,
            password: None,
            resolve_target: p.default_resolve_target(),
            no_proxy: None,
        }
    }

    fn new_with_flag(
        proxy: &str,
        no_proxy: Option<NoProxy>,
        from_env: bool,
        resolve_target: Option<bool>,
    ) -> Result<Self, Error> {
        let mut uri = proxy.parse::<Uri>().or(Err(Error::InvalidProxyUrl))?;

        // The uri must have an authority part (with the host), or
        // it is invalid.
        let _ = uri.authority().ok_or(Error::InvalidProxyUrl)?;

        let scheme = match uri.scheme_str() {
            Some(v) => v,
            None => {
                // The default protocol is Proto::HTTP, and it is missing in
                // the uri. Let's put it in place.
                uri = insert_default_scheme(uri);
                "http"
            }
        };

        let proto: ProxyProtocol = scheme.try_into()?;
        let resolve_target = resolve_target.unwrap_or(proto.default_resolve_target());

        let inner = ProxyInner {
            proto,
            uri,
            from_env,
            resolve_target,
            no_proxy,
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
    /// Additionally, the `NO_PROXY` environment variable is automatically read to determine
    /// which hosts should bypass the proxy. This supports various pattern types including
    /// exact hostnames, wildcard suffixes, and dot suffixes.
    ///
    /// Returns `None` if no environment variable is set or the URI is invalid.
    pub fn try_from_env() -> Option<Self> {
        const TRY_ENV: &[&str] = &[
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ];

        let no_proxy = NoProxy::try_from_env();
        for attempt in TRY_ENV {
            if let Ok(env) = std::env::var(attempt) {
                if let Ok(proxy) = Self::new_with_flag(&env, no_proxy.clone(), true, None) {
                    return Some(proxy);
                }
            }
        }

        #[cfg(all(windows, feature = "win-system-proxy"))]
        {
            use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
            use winreg::RegKey;

            let registry = RegKey::predef(HKEY_CURRENT_USER);
            let Ok(ie_settings) = registry.open_subkey_with_flags(REGISTRY_PATH, KEY_READ) else {
                return None;
            };

            let enabled = ie_settings
                .get_value::<u32, _>("ProxyEnable")
                .is_ok_and(|enable| enable == 1);
            if !enabled {
                return None;
            }

            ie_settings
                .get_value::<String, _>("ProxyServer")
                .ok()
                .and_then(|proxy| {
                    Self::new_with_flag(&format!("http://{proxy}"), no_proxy, true, None).ok()
                })
        }
        #[cfg(not(all(windows, feature = "win-system-proxy")))]
        None
    }

    /// The configured protocol.
    pub fn protocol(&self) -> ProxyProtocol {
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

    /// Whether to resolve target locally before calling the proxy.
    ///
    /// * `true` - resolve the DNS before calling proxy.
    /// * `false` - send the target host to the proxy and let it resolve.
    ///
    /// Defaults to `false` for all proxies protocols except `SOCKS4`. I.e. the normal
    /// case is to let the proxy resolve the target host.
    pub fn resolve_target(&self) -> bool {
        self.inner.resolve_target
    }

    /// Tells if this entry matches anything on the NO_PROXY list.
    ///
    /// This method is used by Proxy Connectors to decide if a connection to the given host
    /// should be routed through the proxy or established directly.
    ///
    /// * `false` - The connection should be routed through the proxy connector
    /// * `true` - The connection should bypass the proxy and connect directly to the host
    pub fn is_no_proxy(&self, uri: &Uri) -> bool {
        if let (Some(no_proxy), Some(host)) = (&self.inner.no_proxy, uri.host()) {
            return no_proxy.is_no_proxy(host);
        }
        false
    }
}

fn insert_default_scheme(uri: Uri) -> Uri {
    let mut parts = uri.into_parts();

    parts.scheme = Some(Scheme::HTTP);

    // For some reason uri.into_parts can produce None for
    // the path, but Uri::from_parts does not accept that.
    parts.path_and_query = parts
        .path_and_query
        .or_else(|| Some(PathAndQuery::from_static("/")));

    Uri::from_parts(parts).unwrap()
}

/// Builder for configuring a proxy.
///
/// Obtained via [`Proxy::builder()`].
pub struct ProxyBuilder {
    protocol: ProxyProtocol,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    resolve_target: bool,
    no_proxy: Option<NoProxy>,
}

impl ProxyBuilder {
    /// Set the proxy hostname
    ///
    /// Defaults to `localhost`. Invalid hostnames surface in [`ProxyBuilder::build()`].
    pub fn host(mut self, host: &str) -> Self {
        self.host = Some(host.to_string());
        self
    }

    /// Set the proxy port
    ///
    /// Defaults to whatever is default for the chosen [`ProxyProtocol`].
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set the username
    ///
    /// Defaults to none. Invalid usernames surface in [`ProxyBuilder::build()`].
    pub fn username(mut self, v: &str) -> Self {
        self.username = Some(v.to_string());
        self
    }

    /// Set the password
    ///
    /// If you want to set only a password, no username, i.e. `https://secret@foo.com`,
    /// you need to set it as [`ProxyBuilder::username()`].
    ///
    /// Defaults to none.  Invalid passwords surface in [`ProxyBuilder::build()`].
    pub fn password(mut self, v: &str) -> Self {
        self.password = Some(v.to_string());
        self
    }

    /// Whether to resolve the target host locally before calling the proxy.
    ///
    /// * `true` - resolve target host locally before calling proxy.
    /// * `false` - let proxy resolve the host.
    ///
    /// For SOCKS4, this defaults to `true`, for all other protocols `false`. I.e.
    /// in the "normal" case, we let the proxy itself resolve host names.
    pub fn resolve_target(mut self, do_resolve: bool) -> Self {
        self.resolve_target = do_resolve;
        self
    }

    /// Add a NO_PROXY expression to not route proxy through.
    ///
    /// Correct expressions are:
    ///
    /// * `example.com` -> Literally match `example.com`, but not `sub.example.com`
    /// * `.example.com` -> Match `sub.example.com` and `foo.sub.example.com`, but not `example.com`.
    /// * `*.example.com` -> Exactly like `.example.com`
    /// * `*` -> Match everything
    ///
    /// Silently ignores expressions that are not on the above form.
    pub fn no_proxy(mut self, expr: &str) -> Self {
        if let Some(entry) = NoProxyEntry::try_parse(expr) {
            if self.no_proxy.is_none() {
                self.no_proxy = Some(NoProxy::default());
            }
            self.no_proxy.as_mut().unwrap().inner.push(entry);
        }

        self
    }

    /// Construct the [`Proxy`]
    pub fn build(self) -> Result<Proxy, Error> {
        let host = self.host.as_deref().unwrap_or("localhost");
        let port = self.port.unwrap_or(self.protocol.default_port());

        let mut userpass = String::new();
        if let Some(username) = self.username {
            userpass.push_str(&username);
            if let Some(password) = self.password {
                userpass.push(':');
                userpass.push_str(&password);
            }
            userpass.push('@');
        }

        // TODO(martin): This incurs as a somewhat unnecessary allocation, but we get some
        // validation and normalization in new_with_flag. This could be refactored
        // in the future.
        let proxy = format!("{}://{}{}:{}", self.protocol, userpass, host, port);
        Proxy::new_with_flag(&proxy, self.no_proxy, false, Some(self.resolve_target))
    }
}

impl TryFrom<&str> for ProxyProtocol {
    type Error = Error;

    fn try_from(scheme: &str) -> Result<Self, Self::Error> {
        match scheme.to_ascii_lowercase().as_str() {
            "http" => Ok(ProxyProtocol::Http),
            "https" => Ok(ProxyProtocol::Https),
            "socks4" => Ok(ProxyProtocol::Socks4),
            "socks4a" => Ok(ProxyProtocol::Socks4A),
            "socks" => Ok(ProxyProtocol::Socks5),
            "socks5" => Ok(ProxyProtocol::Socks5),
            "socks5h" => Ok(ProxyProtocol::Socks5h),
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

impl fmt::Display for ProxyProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyProtocol::Http => write!(f, "HTTP"),
            ProxyProtocol::Https => write!(f, "HTTPS"),
            ProxyProtocol::Socks4 => write!(f, "SOCKS4"),
            ProxyProtocol::Socks4A => write!(f, "SOCKS4a"),
            ProxyProtocol::Socks5 => write!(f, "SOCKS5"),
            ProxyProtocol::Socks5h => write!(f, "SOCKS5h"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum NoProxyEntry {
    ExactHost(String),
    HostPrefix(String),
    HostSuffix(String),
    MatchAll,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Default)]
struct NoProxy {
    inner: Vec<NoProxyEntry>,
}

impl NoProxy {
    /// Read no proxy settings from environment variables.
    ///
    /// The environment variable is expected to contain values separated by comma. The following
    /// environment variables are attempted:
    ///
    /// * `NO_PROXY`
    /// * `no_proxy`
    ///
    /// ## Supported Pattern Types
    ///
    /// * **Exact match**: `localhost`, `127.0.0.1` - matches the exact hostname (case-insensitive)
    /// * **Wildcard suffix**: `*.example.com` - matches any subdomain of example.com
    /// * **Dot suffix**: `.example.com` - matches any subdomain of example.com (but not example.com itself)
    /// * **Match all**: `*` - bypasses proxy for all requests
    ///
    /// ## Examples
    ///
    /// ```bash
    /// # Bypass proxy for localhost and internal domains
    /// export NO_PROXY=localhost,127.0.0.1,*.internal.com
    ///
    /// # Bypass proxy for staging subdomains but not staging itself
    /// export NO_PROXY=.staging
    ///
    /// # Bypass proxy for everything
    /// export NO_PROXY=*
    /// ```
    ///
    /// Returns `None` if no environment variable is set
    pub fn try_from_env() -> Option<Self> {
        const TRY_ENV: &[&str] = &["NO_PROXY", "no_proxy"];

        for attempt in TRY_ENV {
            if let Ok(env) = std::env::var(attempt) {
                let inner = env.split(',').filter_map(NoProxyEntry::try_parse).collect();
                return Some(Self { inner });
            }
        }

        #[cfg(all(windows, feature = "win-system-proxy"))]
        {
            use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
            use winreg::RegKey;

            let registry = RegKey::predef(HKEY_CURRENT_USER);
            let Ok(ie_settings) = registry.open_subkey_with_flags(REGISTRY_PATH, KEY_READ) else {
                return None;
            };

            ie_settings
                .get_value::<String, _>("ProxyOverride")
                .ok()
                .map(|no_proxy| NoProxy {
                    inner: no_proxy
                        .split(";")
                        .map(str::trim)
                        // bypass <local>, which tells windows to bypass intranet addresses
                        .filter(|&s| s != "<local>")
                        .map(NoProxyEntry::try_parse)
                        .flatten()
                        .collect(),
                })
        }
        #[cfg(not(all(windows, feature = "win-system-proxy")))]
        None
    }

    pub fn is_no_proxy(&self, host: &str) -> bool {
        self.inner.iter().any(|entry| entry.matches(host))
    }
}

impl NoProxyEntry {
    fn try_parse(u: &str) -> Option<Self> {
        let entry = match u {
            "*" => Self::MatchAll,
            u if u.starts_with("*") => {
                Self::HostSuffix(u.chars().skip(1).collect::<String>().to_ascii_lowercase())
            }
            u if u.starts_with(".") => Self::HostSuffix(u.to_ascii_lowercase()),
            u if u.ends_with("*") => Self::HostPrefix(
                u.chars()
                    .take(u.len() - 1)
                    .collect::<String>()
                    .to_ascii_lowercase(),
            ),
            u if u.ends_with(".") => Self::HostPrefix(u.to_ascii_lowercase()),
            _ => Self::ExactHost(u.to_ascii_lowercase()),
        };
        Some(entry)
    }

    fn matches(&self, host: &str) -> bool {
        match self {
            NoProxyEntry::MatchAll => true,
            NoProxyEntry::ExactHost(pattern) => {
                // Fast path: if host is already lowercase, do direct comparison
                if host.chars().all(|c| !c.is_ascii_uppercase()) {
                    pattern == host
                } else {
                    // Slow path: convert host to lowercase and compare
                    pattern == &host.to_ascii_lowercase()
                }
            }
            NoProxyEntry::HostPrefix(prefix) => {
                if host.len() < prefix.len() {
                    return false;
                }
                let host_prefix = &host[..prefix.len()];
                // Fast path: if host prefix is already lowercase, do direct comparison
                if host_prefix.chars().all(|c| !c.is_ascii_uppercase()) {
                    prefix == host_prefix
                } else {
                    // Slow path: convert host prefix to lowercase and compare
                    prefix == &host_prefix.to_ascii_lowercase()
                }
            }
            NoProxyEntry::HostSuffix(suffix) => {
                if host.len() < suffix.len() {
                    return false;
                }
                let host_suffix = &host[host.len() - suffix.len()..];
                // Fast path: if host suffix is already lowercase, do direct comparison
                if host_suffix.chars().all(|c| !c.is_ascii_uppercase()) {
                    suffix == host_suffix
                } else {
                    // Slow path: convert host suffix to lowercase and compare
                    suffix == &host_suffix.to_ascii_lowercase()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use assert_no_alloc::*;
    use std::str::FromStr;

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
        assert_eq!(proxy.inner.proto, ProxyProtocol::Http);
    }

    #[test]
    fn parse_proxy_http_user_pass_server_port_trailing_slash() {
        let proxy = Proxy::new("http://user:p@ssw0rd@localhost:9999/").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Http);
    }

    #[test]
    fn parse_proxy_socks4_user_pass_server_port() {
        let proxy = Proxy::new("socks4://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Socks4);
        assert!(proxy.resolve_target());
    }

    #[test]
    fn parse_proxy_socks4a_user_pass_server_port() {
        let proxy = Proxy::new("socks4a://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Socks4A);
        assert!(!proxy.resolve_target());
    }

    #[test]
    fn parse_proxy_socks_user_pass_server_port() {
        let proxy = Proxy::new("socks://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Socks5);
        assert!(proxy.resolve_target());
    }

    #[test]
    fn parse_proxy_socks5_user_pass_server_port() {
        let proxy = Proxy::new("socks5://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Socks5);
        assert!(proxy.resolve_target());
    }

    #[test]
    fn parse_proxy_socks5h_user_pass_server_port() {
        let proxy = Proxy::new("socks5h://user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Socks5h);
        assert!(!proxy.resolve_target());
    }

    #[test]
    fn parse_proxy_user_pass_server_port() {
        let proxy = Proxy::new("user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.username(), Some("user"));
        assert_eq!(proxy.password(), Some("p@ssw0rd"));
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Http);
    }

    #[test]
    fn parse_proxy_server_port() {
        let proxy = Proxy::new("localhost:9999").unwrap();
        assert_eq!(proxy.username(), None);
        assert_eq!(proxy.password(), None);
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 9999);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Http);
    }

    #[test]
    fn parse_proxy_server() {
        let proxy = Proxy::new("localhost").unwrap();
        assert_eq!(proxy.username(), None);
        assert_eq!(proxy.password(), None);
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 80);
        assert_eq!(proxy.inner.proto, ProxyProtocol::Http);
    }

    #[test]
    fn no_proxy_exact_host_matching() {
        let p = Proxy::builder(ProxyProtocol::Http)
            .host("proxy.example.com")
            .port(8080)
            .no_proxy("localhost")
            .no_proxy("127.0.0.1")
            .no_proxy("api.internal.com")
            .build()
            .unwrap();

        fn is_no_proxy(p: &Proxy, host: &str) -> bool {
            let uri = Uri::from_str(&format!("http://{}", host)).unwrap();
            p.is_no_proxy(&uri)
        }

        // Should match exact hosts
        assert!(is_no_proxy(&p, "localhost"));
        assert!(is_no_proxy(&p, "127.0.0.1"));
        assert!(is_no_proxy(&p, "api.internal.com"));

        // Should not match partial or different hosts
        assert!(!is_no_proxy(&p, "mylocalhost"));
        assert!(!is_no_proxy(&p, "localhost.example.com"));
        assert!(!is_no_proxy(&p, "127.0.0.2"));
        assert!(!is_no_proxy(&p, "api.internal.com.evil.com"));
        assert!(!is_no_proxy(&p, "docs.rs"));
    }

    #[test]
    fn no_proxy_wildcard_suffix_matching() {
        let p = Proxy::builder(ProxyProtocol::Http)
            .host("proxy.example.com")
            .port(8080)
            .no_proxy("*.internal.com")
            .no_proxy("*.dev")
            .build()
            .unwrap();

        fn is_no_proxy(p: &Proxy, host: &str) -> bool {
            let uri = Uri::from_str(&format!("http://{}", host)).unwrap();
            p.is_no_proxy(&uri)
        }

        // Should match wildcard suffixes
        assert!(is_no_proxy(&p, "api.internal.com"));
        assert!(is_no_proxy(&p, "auth.internal.com"));
        assert!(is_no_proxy(&p, "db.internal.com"));
        assert!(is_no_proxy(&p, "app.dev"));
        assert!(is_no_proxy(&p, "test.dev"));

        // Should not match the bare suffix or unrelated hosts
        assert!(!is_no_proxy(&p, "internal.com"));
        assert!(!is_no_proxy(&p, "dev"));
        assert!(!is_no_proxy(&p, "api.external.com"));
        assert!(!is_no_proxy(&p, "app.prod"));
        assert!(!is_no_proxy(&p, "docs.rs"));
    }

    #[test]
    fn no_proxy_dot_suffix_matching() {
        let p = Proxy::builder(ProxyProtocol::Http)
            .host("proxy.example.com")
            .port(8080)
            .no_proxy(".internal.com")
            .no_proxy(".staging")
            .build()
            .unwrap();

        fn is_no_proxy(p: &Proxy, host: &str) -> bool {
            let uri = Uri::from_str(&format!("http://{}", host)).unwrap();
            p.is_no_proxy(&uri)
        }

        // Should match dot suffix patterns (only subdomains, not the domain itself)
        assert!(is_no_proxy(&p, "api.internal.com"));
        assert!(is_no_proxy(&p, "auth.internal.com"));
        assert!(is_no_proxy(&p, "db.sub.internal.com"));
        assert!(is_no_proxy(&p, "app.staging"));
        assert!(is_no_proxy(&p, "test.staging"));

        // Should NOT match the bare domain (key difference from wildcard)
        assert!(!is_no_proxy(&p, "internal.com"));
        assert!(!is_no_proxy(&p, "staging"));

        // Should not match unrelated hosts
        assert!(!is_no_proxy(&p, "api.external.com"));
        assert!(!is_no_proxy(&p, "prod"));
        assert!(!is_no_proxy(&p, "docs.rs"));
    }

    #[test]
    fn no_proxy_match_all_wildcard() {
        let p = Proxy::builder(ProxyProtocol::Http)
            .host("proxy.example.com")
            .port(8080)
            .no_proxy("*")
            .build()
            .unwrap();

        fn is_no_proxy(p: &Proxy, host: &str) -> bool {
            let uri = Uri::from_str(&format!("http://{}", host)).unwrap();
            p.is_no_proxy(&uri)
        }

        // Should match everything when using "*"
        assert!(is_no_proxy(&p, "localhost"));
        assert!(is_no_proxy(&p, "127.0.0.1"));
        assert!(is_no_proxy(&p, "api.example.com"));
        assert!(is_no_proxy(&p, "docs.rs"));
        assert!(is_no_proxy(&p, "github.com"));
        assert!(is_no_proxy(&p, "any.random.domain"));
    }

    #[test]
    fn no_proxy_mixed_patterns() {
        let p = Proxy::builder(ProxyProtocol::Http)
            .host("proxy.example.com")
            .port(8080)
            .no_proxy("localhost") // exact host
            .no_proxy("*.dev") // wildcard suffix
            .no_proxy(".staging") // dot suffix
            .no_proxy("127.0.0.1") // exact IP
            .build()
            .unwrap();

        fn is_no_proxy(p: &Proxy, host: &str) -> bool {
            let uri = Uri::from_str(&format!("http://{}", host)).unwrap();
            p.is_no_proxy(&uri)
        }

        // Should match exact hosts
        assert!(is_no_proxy(&p, "localhost"));
        assert!(is_no_proxy(&p, "127.0.0.1"));

        // Should match wildcard suffixes
        assert!(is_no_proxy(&p, "api.dev"));
        assert!(is_no_proxy(&p, "test.dev"));

        // Should match dot suffixes (only subdomains, not the domain itself)
        assert!(is_no_proxy(&p, "app.staging"));
        assert!(!is_no_proxy(&p, "staging"));

        // Should not match unrelated hosts
        assert!(!is_no_proxy(&p, "dev")); // bare wildcard suffix
        assert!(!is_no_proxy(&p, "api.prod")); // different suffix
        assert!(!is_no_proxy(&p, "docs.rs")); // unrelated
        assert!(!is_no_proxy(&p, "127.0.0.2")); // different IP
    }

    #[test]
    fn no_proxy_case_insensitive_matching() {
        let p = Proxy::builder(ProxyProtocol::Http)
            .host("proxy.example.com")
            .port(8080)
            .no_proxy("localhost")
            .no_proxy("*.Example.Com")
            .no_proxy(".INTERNAL")
            .build()
            .unwrap();

        fn is_no_proxy(p: &Proxy, host: &str) -> bool {
            let uri = Uri::from_str(&format!("http://{}", host)).unwrap();
            p.is_no_proxy(&uri)
        }

        // Test exact host matching - should be case insensitive
        // These patterns are stored as lowercase: "localhost"
        assert!(is_no_proxy(&p, "localhost")); // fast path: already lowercase
        assert!(is_no_proxy(&p, "LOCALHOST")); // slow path: needs conversion
        assert!(is_no_proxy(&p, "LocalHost")); // slow path: needs conversion

        // Test wildcard suffix case insensitive matching
        // These patterns are stored as lowercase: ".example.com"
        assert!(is_no_proxy(&p, "api.example.com")); // fast path: already lowercase
        assert!(is_no_proxy(&p, "api.EXAMPLE.COM")); // slow path: needs conversion
        assert!(is_no_proxy(&p, "API.example.COM")); // slow path: needs conversion
        assert!(is_no_proxy(&p, "api.Example.Com")); // slow path: needs conversion

        // Test dot suffix case insensitive matching (only matches subdomains)
        // These patterns are stored as lowercase: ".internal"
        assert!(is_no_proxy(&p, "app.internal")); // fast path: already lowercase
        assert!(is_no_proxy(&p, "app.INTERNAL")); // slow path: needs conversion
        assert!(is_no_proxy(&p, "APP.Internal")); // slow path: needs conversion
        assert!(!is_no_proxy(&p, "INTERNAL")); // bare domain doesn't match dot suffix
        assert!(!is_no_proxy(&p, "internal")); // bare domain doesn't match dot suffix
    }

    #[test]
    fn no_proxy_edge_cases() {
        let p = Proxy::builder(ProxyProtocol::Http)
            .host("proxy.example.com")
            .port(8080)
            .no_proxy("") // empty string
            .no_proxy("single") // single word
            .no_proxy("*..") // malformed wildcard
            .no_proxy("..") // malformed dot suffix
            .no_proxy("192.168.1.1") // IP address
            .no_proxy("*.local") // local domain
            .build()
            .unwrap();

        fn is_no_proxy(p: &Proxy, host: &str) -> bool {
            let uri = Uri::from_str(&format!("http://{}", host)).unwrap();
            p.is_no_proxy(&uri)
        }

        // Test exact matching of various formats
        assert!(is_no_proxy(&p, "single"));
        assert!(is_no_proxy(&p, "192.168.1.1"));
        assert!(!is_no_proxy(&p, "192.168.1.2"));

        // Test wildcard with local domains
        assert!(is_no_proxy(&p, "printer.local"));
        assert!(is_no_proxy(&p, "router.local"));
        assert!(!is_no_proxy(&p, "local")); // bare domain

        // Test that malformed patterns don't break things
        assert!(is_no_proxy(&p, "something..")); // matches exactly
        assert!(!is_no_proxy(&p, "something.else"));

        // Test empty string exact match
        // Note: This is likely an edge case that shouldn't happen in practice
        // but we want to ensure it doesn't crash
    }

    #[test]
    fn proxy_clone_does_not_allocate() {
        let c = Proxy::new("socks://1.2.3.4").unwrap();
        assert_no_alloc(|| c.clone());
    }

    #[test]
    fn proxy_new_default_scheme() {
        let c = Proxy::new("localhost:1234").unwrap();
        assert_eq!(c.protocol(), ProxyProtocol::Http);
        assert_eq!(c.uri(), "http://localhost:1234");
    }

    #[test]
    fn proxy_empty_env_url() {
        let result = Proxy::new_with_flag("", None, false, None);
        assert!(result.is_err());
    }

    #[test]
    fn proxy_invalid_env_url() {
        let result = Proxy::new_with_flag("r32/?//52:**", None, false, None);
        assert!(result.is_err());
    }

    #[test]
    fn proxy_builder() {
        let proxy = Proxy::builder(ProxyProtocol::Socks4)
            .host("my-proxy.com")
            .port(5551)
            .resolve_target(false)
            .build()
            .unwrap();

        assert_eq!(proxy.protocol(), ProxyProtocol::Socks4);
        assert_eq!(proxy.uri(), "SOCKS4://my-proxy.com:5551/");
        assert_eq!(proxy.host(), "my-proxy.com");
        assert_eq!(proxy.port(), 5551);
        assert_eq!(proxy.username(), None);
        assert_eq!(proxy.password(), None);
        assert_eq!(proxy.is_from_env(), false);
        assert_eq!(proxy.resolve_target(), false);
    }

    #[test]
    fn proxy_builder_username() {
        let proxy = Proxy::builder(ProxyProtocol::Https)
            .username("hemligearne")
            .build()
            .unwrap();

        assert_eq!(proxy.protocol(), ProxyProtocol::Https);
        assert_eq!(proxy.uri(), "https://hemligearne@localhost:443/");
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 443);
        assert_eq!(proxy.username(), Some("hemligearne"));
        assert_eq!(proxy.password(), None);
        assert_eq!(proxy.is_from_env(), false);
        assert_eq!(proxy.resolve_target(), false);
    }

    #[test]
    fn proxy_builder_username_password() {
        let proxy = Proxy::builder(ProxyProtocol::Https)
            .username("hemligearne")
            .password("kulgrej")
            .build()
            .unwrap();

        assert_eq!(proxy.protocol(), ProxyProtocol::Https);
        assert_eq!(proxy.uri(), "https://hemligearne:kulgrej@localhost:443/");
        assert_eq!(proxy.host(), "localhost");
        assert_eq!(proxy.port(), 443);
        assert_eq!(proxy.username(), Some("hemligearne"));
        assert_eq!(proxy.password(), Some("kulgrej"));
        assert_eq!(proxy.is_from_env(), false);
        assert_eq!(proxy.resolve_target(), false);
    }
}
