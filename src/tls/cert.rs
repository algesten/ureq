use crate::Error;

pub struct Certificate<'a> {
    der: &'a [u8],
}

impl<'a> Certificate<'a> {
    pub fn from_der(der: &'a [u8]) -> Self {
        Certificate { der }
    }

    pub fn from_pem(pem: &'a [u8]) -> Result<Self, Error> {
        let item = parse_pem(pem)
            .find(|p| matches!(p, Err(_) | Ok(PemItem::Certificate(_))))
            // None means there were no matches in the PEM chain
            .ok_or(Error::Certificate("no pem encoded cert found"))??;

        let cert = match item {
            PemItem::Certificate(v) => v,
            PemItem::PrivateKey(_) => unreachable!("matches! above for Certificate"),
        };

        Ok(cert)
    }

    pub fn der(&self) -> &[u8] {
        self.der
    }
}

pub struct PrivateKey<'a> {
    kind: KeyKind,
    der: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Pkcs1,
    Pkcs8,
    Sec1,
}

impl<'a> PrivateKey<'a> {
    pub fn from_der(kind: KeyKind, der: &'a [u8]) -> Self {
        PrivateKey { kind, der }
    }

    pub fn from_pem(pem: &'a [u8]) -> Result<Self, Error> {
        let item = parse_pem(pem)
            .find(|p| matches!(p, Err(_) | Ok(PemItem::PrivateKey(_))))
            // None means there were no matches in the PEM chain
            .ok_or(Error::Certificate("no pem encoded private key found"))??;

        let key = match item {
            PemItem::PrivateKey(v) => v,
            PemItem::Certificate(_) => unreachable!("matches! above for PrivateKey"),
        };

        Ok(key)
    }

    pub fn kind(&self) -> KeyKind {
        self.kind
    }

    pub fn der(&self) -> &[u8] {
        self.der
    }
}

pub enum PemItem<'a> {
    Certificate(Certificate<'a>),
    PrivateKey(PrivateKey<'a>),
}

pub fn parse_pem(pem: &[u8]) -> impl Iterator<Item = Result<PemItem, Error>> + '_ {
    PemIter(pem)
}

struct PemIter<'a>(&'a [u8]);

impl<'a> Iterator for PemIter<'a> {
    type Item = Result<PemItem<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match rustls_pemfile::read_one_from_slice(&self.0) {
                Ok(Some((cert, rest))) => {
                    // A bit backwards engineering to figure out which part of the input
                    // was parsed to an complete item.
                    let remaining = rest.len();
                    let parsed_len = self.0.len() - remaining;
                    let der = &self.0[..parsed_len];

                    // Move slice along for next iterator next()
                    self.0 = rest;

                    match cert {
                        rustls_pemfile::Item::X509Certificate(_) => {
                            return Some(Ok(Certificate::from_der(der).into()));
                        }
                        rustls_pemfile::Item::Pkcs1Key(_) => {
                            return Some(Ok(PrivateKey::from_der(KeyKind::Pkcs1, der).into()));
                        }
                        rustls_pemfile::Item::Pkcs8Key(_) => {
                            return Some(Ok(PrivateKey::from_der(KeyKind::Pkcs8, der).into()));
                        }
                        rustls_pemfile::Item::Sec1Key(_) => {
                            return Some(Ok(PrivateKey::from_der(KeyKind::Sec1, der).into()));
                        }

                        // Skip unhandled item type (CSR etc)
                        _ => continue,
                    }
                }

                // It's over
                Ok(None) => return None,

                Err(e) => {
                    warn!("bad pem encoded cert: {:?}", e);
                    return Some(Err(Error::Certificate("bad pem encoded cert")));
                }
            }
        }
    }
}

impl<'a> From<Certificate<'a>> for PemItem<'a> {
    fn from(value: Certificate<'a>) -> Self {
        PemItem::Certificate(value)
    }
}

impl<'a> From<PrivateKey<'a>> for PemItem<'a> {
    fn from(value: PrivateKey<'a>) -> Self {
        PemItem::PrivateKey(value)
    }
}