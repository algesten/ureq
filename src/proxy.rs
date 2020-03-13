use crate::error::Error;

/// Kind of proxy connection (Basic, Digest, etc)
#[derive(Clone, Debug)]
pub(crate) enum ProxyKind {
    Basic,
}

/// Proxy server definition
#[derive(Clone, Debug)]
pub struct Proxy {
    pub(crate) server: String,
    pub(crate) port: u32,
    pub(crate) user: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) kind: ProxyKind,
}

impl Proxy {
    fn parse_creds<S: AsRef<str>>(
        creds: &Option<S>,
    ) -> Result<(Option<String>, Option<String>), Error> {
        match creds {
            Some(creds) => {
                let mut parts = creds
                    .as_ref()
                    .splitn(2, ':')
                    .collect::<Vec<&str>>()
                    .into_iter();

                if parts.len() != 2 {
                    Err(Error::BadProxyCreds)
                } else {
                    Ok((
                        parts.next().map(String::from),
                        parts.next().map(String::from),
                    ))
                }
            }
            None => Ok((None, None)),
        }
    }

    fn parse_address<S: AsRef<str>>(host: &Option<S>) -> Result<(String, Option<u32>), Error> {
        match host {
            Some(host) => {
                let mut parts = host.as_ref().split(':').collect::<Vec<&str>>().into_iter();
                let host = parts.next().ok_or(Error::BadProxy)?;
                let port = parts.next();
                Ok((
                    String::from(host),
                    port.and_then(|port| port.parse::<u32>().ok()),
                ))
            }
            None => Err(Error::BadProxy),
        }
    }

    fn use_authorization(&self) -> bool {
        self.user.is_some() && self.password.is_some()
    }

    pub fn new<S: AsRef<str>>(proxy: S) -> Result<Self, Error> {
        let mut parts = proxy
            .as_ref()
            .rsplitn(2, '@')
            .collect::<Vec<&str>>()
            .into_iter()
            .rev();

        let (user, password) = if parts.len() == 2 {
            Proxy::parse_creds(&parts.next())?
        } else {
            (None, None)
        };

        let (server, port) = Proxy::parse_address(&parts.next())?;

        Ok(Self {
            server,
            user,
            password,
            port: port.unwrap_or(8080),
            kind: ProxyKind::Basic,
        })
    }

    pub fn connect<S: AsRef<str>>(&self, host: S, port: u16) -> String {
        let authorization = if self.use_authorization() {
            match self.kind {
                ProxyKind::Basic => {
                    let creds = base64::encode(&format!(
                        "{}:{}",
                        self.user.clone().unwrap_or_default(),
                        self.password.clone().unwrap_or_default()
                    ));
                    format!("Proxy-Authorization: basic {}\r\n", creds)
                }
            }
        } else {
            String::new()
        };

        format!(
            "CONNECT {}:{} HTTP/1.1\r\n\
Host: {}:{}\r\n\
User-Agent: something/1.0.0\r\n\
Proxy-Connection: Keep-Alive\r\n\
{}\
\r\n",
            host.as_ref(),
            port,
            host.as_ref(),
            port,
            authorization
        )
    }

    pub(crate) fn verify_response(response: &[u8]) -> Result<(), Error> {
        let response_string = String::from_utf8_lossy(response);
        let top_line = response_string.lines().next().ok_or(Error::ProxyConnect)?;
        let status_code = top_line.split_whitespace().nth(1).ok_or(Error::BadProxy)?;

        match status_code {
            "200" => Ok(()),
            "401" | "407" => Err(Error::InvalidProxyCreds),
            _ => Err(Error::BadProxy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Proxy;

    #[test]
    fn parse_proxy() {
        let proxy = Proxy::new("user:p@ssw0rd@localhost:9999").unwrap();
        assert_eq!(proxy.user, Some(String::from("user")));
        assert_eq!(proxy.password, Some(String::from("p@ssw0rd")));
        assert_eq!(proxy.server, String::from("localhost"));
        assert_eq!(proxy.port, 9999);
    }
}
