use ureq_proto::http::Uri;

/// Helps us match hostnames to patterns - mainly used for NO_PROXY support.
#[derive(Clone, Debug)]
pub enum HostnameMatcher {
    /// Matches the pattern literally - by string equality
    Literal(String),
    /// Subdomain match - equivalent to checking if ther pattern is the suffix to the string
    Suffix(String),
    /// Matches any string
    MatchAll,
}

impl HostnameMatcher {
    pub fn parse(pattern: &str) -> Self {
        if pattern == "*" {
            return Self::MatchAll;
        }

        let pattern = pattern.to_owned();

        if pattern.starts_with('.') {
            return Self::Suffix(pattern);
        }
        Self::Literal(pattern)
    }

    pub fn matches(&self, uri: &Uri) -> bool {
        let Some(hostname) = uri.host() else {
            return false;
        };

        match self {
            Self::Literal(lit) => lit == hostname,
            Self::Suffix(suffix) => hostname.ends_with(suffix),
            Self::MatchAll => true,
        }
    }
}
