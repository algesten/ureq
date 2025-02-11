use std::fmt;
use std::hash::{Hash, Hasher};

use crate::Error;

/// An X509 certificate for a server or a client.
///
/// These are either used as trust roots, or client authentication.
///
/// The internal representation is DER form. The provided helpers for PEM
/// translates to DER.
#[derive(Clone, Hash)]
pub struct Certificate<'a> {
    der: CertDer<'a>,
}

#[derive(Clone)]
enum CertDer<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
    Rustls(rustls_pki_types::CertificateDer<'static>),
}

impl Hash for CertDer<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        self.as_ref().hash(state)
    }
}

impl<'a> AsRef<[u8]> for CertDer<'a> {
    fn as_ref(&self) -> &[u8] {
        match self {
            CertDer::Borrowed(v) => v,
            CertDer::Owned(v) => v,
            CertDer::Rustls(v) => v,
        }
    }
}

impl<'a> Certificate<'a> {
    /// Read an X509 certificate in DER form.
    ///
    /// Does not immediately validate whether the data provided is a valid DER formatted
    /// X509. That validation is the responsibility of the TLS provider.
    pub fn from_der(der: &'a [u8]) -> Self {
        let der = CertDer::Borrowed(der);
        Certificate { der }
    }

    /// Read an X509 certificate in PEM form.
    ///
    /// This is a shorthand for [`parse_pem`] followed by picking the first certificate.
    /// Fails with an error if there is no certificate found in the PEM given.
    ///
    /// Translates to DER format internally.
    pub fn from_pem(pem: &'a [u8]) -> Result<Certificate<'static>, Error> {
        let item = parse_pem(pem)
            .find(|p| matches!(p, Err(_) | Ok(PemItem::Certificate(_))))
            // None means there were no matches in the PEM chain
            .ok_or(Error::Tls("No pem encoded cert found"))??;

        let PemItem::Certificate(cert) = item else {
            unreachable!("matches! above for Certificate");
        };

        Ok(cert)
    }

    /// This certificate in DER (the internal) format.
    pub fn der(&self) -> &[u8] {
        self.der.as_ref()
    }

    /// Clones (allocates) to produce a static copy.
    pub fn to_owned(&self) -> Certificate<'static> {
        Certificate {
            der: CertDer::Owned(self.der.as_ref().to_vec()),
        }
    }
}

/// A private key used in client certificate auth.
///
/// The internal representation is DER form. The provided helpers for PEM
/// translates to DER.
///
/// Deliberately not `Clone` to avoid accidental copies in memory.
#[derive(Hash)]
pub struct PrivateKey<'a> {
    kind: KeyKind,
    der: PrivateKeyDer<'a>,
}

enum PrivateKeyDer<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
    Rustls(rustls_pki_types::PrivateKeyDer<'static>),
}

impl Hash for PrivateKeyDer<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            PrivateKeyDer::Borrowed(v) => v.hash(state),
            PrivateKeyDer::Owned(v) => v.hash(state),
            PrivateKeyDer::Rustls(v) => v.secret_der().as_ref().hash(state),
        }
    }
}

impl<'a> AsRef<[u8]> for PrivateKey<'a> {
    fn as_ref(&self) -> &[u8] {
        match &self.der {
            PrivateKeyDer::Borrowed(v) => v,
            PrivateKeyDer::Owned(v) => v,
            PrivateKeyDer::Rustls(v) => v.secret_der(),
        }
    }
}

/// The kind of private key.
///
/// * For **rustls** any kind is valid.
/// * For **native-tls** the only valid option is [`Pkcs8`](KeyKind::Pkcs8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyKind {
    /// An RSA private key
    Pkcs1,
    /// A PKCS#8 private key.
    ///
    /// Not encrypted with a passphrase.
    Pkcs8,
    /// A Sec1 private key
    Sec1,
}

impl<'a> PrivateKey<'a> {
    /// Read private key in DER form.
    ///
    /// Does not immediately validate whether the data provided is a valid DER.
    /// That validation is the responsibility of the TLS provider.
    pub fn from_der(kind: KeyKind, der: &'a [u8]) -> Self {
        let der = PrivateKeyDer::Borrowed(der);
        PrivateKey { kind, der }
    }

    /// Read a private key in PEM form.
    ///
    /// This is a shorthand for [`parse_pem`] followed by picking the first found key.
    /// Fails with an error if there are no keys found in the PEM given.
    ///
    /// Translates to DER format internally.
    pub fn from_pem(pem: &'a [u8]) -> Result<PrivateKey<'static>, Error> {
        let item = parse_pem(pem)
            .find(|p| matches!(p, Err(_) | Ok(PemItem::PrivateKey(_))))
            // None means there were no matches in the PEM chain
            .ok_or(Error::Tls("No pem encoded private key found"))??;

        let PemItem::PrivateKey(key) = item else {
            unreachable!("matches! above for PrivateKey");
        };

        Ok(key)
    }

    /// The key kind
    pub fn kind(&self) -> KeyKind {
        self.kind
    }

    /// This private key in DER (the internal) format.
    pub fn der(&self) -> &[u8] {
        self.as_ref()
    }

    /// Clones (allocates) to produce a static copy.
    pub fn to_owned(&self) -> PrivateKey<'static> {
        PrivateKey {
            kind: self.kind,
            der: match &self.der {
                PrivateKeyDer::Borrowed(v) => PrivateKeyDer::Owned(v.to_vec()),
                PrivateKeyDer::Owned(v) => PrivateKeyDer::Owned(v.to_vec()),
                PrivateKeyDer::Rustls(v) => PrivateKeyDer::Rustls(v.clone_key()),
            },
        }
    }
}

/// Parser of PEM data.
///
/// The data may contain one or many PEM items. The iterator produces the recognized PEM
/// items and skip others.
pub fn parse_pem(pem: &[u8]) -> impl Iterator<Item = Result<PemItem<'static>, Error>> + '_ {
    PemIter(pem)
}

/// Kinds of PEM data found by [`parse_pem`]
#[non_exhaustive]
pub enum PemItem<'a> {
    /// An X509 certificate
    Certificate(Certificate<'a>),

    /// A private key
    PrivateKey(PrivateKey<'a>),
}

struct PemIter<'a>(&'a [u8]);

impl<'a> Iterator for PemIter<'a> {
    type Item = Result<PemItem<'static>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match rustls_pemfile::read_one_from_slice(self.0) {
                Ok(Some((cert, rest))) => {
                    // Move slice along for next iterator next()
                    self.0 = rest;

                    match cert {
                        rustls_pemfile::Item::X509Certificate(der) => {
                            return Some(Ok(Certificate {
                                der: CertDer::Rustls(der),
                            }
                            .into()));
                        }
                        rustls_pemfile::Item::Pkcs1Key(der) => {
                            return Some(Ok(PrivateKey {
                                kind: KeyKind::Pkcs1,
                                der: PrivateKeyDer::Rustls(der.into()),
                            }
                            .into()));
                        }
                        rustls_pemfile::Item::Pkcs8Key(der) => {
                            return Some(Ok(PrivateKey {
                                kind: KeyKind::Pkcs8,
                                der: PrivateKeyDer::Rustls(der.into()),
                            }
                            .into()));
                        }
                        rustls_pemfile::Item::Sec1Key(der) => {
                            return Some(Ok(PrivateKey {
                                kind: KeyKind::Sec1,
                                der: PrivateKeyDer::Rustls(der.into()),
                            }
                            .into()));
                        }

                        // Skip unhandled item type (CSR etc)
                        _ => continue,
                    }
                }

                // It's over
                Ok(None) => return None,

                Err(e) => {
                    return Some(Err(Error::Pem(e)));
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

impl<'a> fmt::Debug for Certificate<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Certificate").finish()
    }
}

impl<'a> fmt::Debug for PrivateKey<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateKey")
            .field("kind", &self.kind)
            .finish()
    }
}
