use ureq_proto::http::Uri;

/// Helps us match hostnames to patterns - mainly used for NO_PROXY support.
#[derive(Clone, Debug)]
pub enum HostnameMatcher {
    /// Matches the pattern literally - by string equality
    Literal(String),
}

impl HostnameMatcher {
    pub fn parse(pattern: &str) -> Self {
        Self::Literal(pattern.to_owned())
    }

    pub fn matches(&self, uri: &Uri) -> bool {
        let Some(hostname) = uri.host() else {
            return false;
        };

        match self {
            Self::Literal(lit) => lit == hostname,
        }
    }
}
