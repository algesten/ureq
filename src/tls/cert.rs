use std::borrow::Cow;
use std::fmt;

use crate::Error;

/// An X509 certificate for a server or a client.
///
/// These are either used as trust roots, or client authentication.
///
/// The internal representation is DER form. The provided helpers for PEM
/// translates to DER.
#[derive(Clone)]
pub struct Certificate<'a> {
    der: CertDer<'a>,
}

#[derive(Clone)]
enum CertDer<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
    // This type is here because rustls_native_certs::load_native_certs() gives us
    // CertificateDer<'static> and we don't want to cause extra allocations.
    #[cfg(feature = "native-roots")]
    PkiTypes(rustls_pki_types::CertificateDer<'a>),
}

impl<'a> AsRef<[u8]> for CertDer<'a> {
    fn as_ref(&self) -> &[u8] {
        match self {
            CertDer::Borrowed(v) => v,
            CertDer::Owned(v) => v,
            #[cfg(feature = "native-roots")]
            CertDer::PkiTypes(v) => v,
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
    pub fn from_pem(pem: &'a [u8]) -> Result<Self, Error> {
        let item = parse_pem(pem)
            .find(|p| matches!(p, Err(_) | Ok(PemItem::Certificate(_))))
            // None means there were no matches in the PEM chain
            .ok_or(Error::Certificate("no pem encoded cert found"))??;

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

    #[cfg(feature = "native-roots")]
    fn from_pki_types(der: rustls_pki_types::CertificateDer<'static>) -> Certificate<'static> {
        Certificate {
            der: CertDer::PkiTypes(der),
        }
    }
}

/// A private key used in client certificate auth.
///
/// The internal representation is DER form. The provided helpers for PEM
/// translates to DER.
///
/// Deliberately not `Clone` to avoid accidental copies in memory.
pub struct PrivateKey<'a> {
    kind: KeyKind,
    der: Cow<'a, [u8]>,
}

/// The kind of private key.
///
/// * For **rustls** any kind is valid.
/// * For **native-tls** the only valid option is [`Pkcs8`](KeyKind::Pkcs8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        let der = Cow::Borrowed(der);
        PrivateKey { kind, der }
    }

    /// Read a private key in PEM form.
    ///
    /// This is a shorthand for [`parse_pem`] followed by picking the first found key.
    /// Fails with an error if there are no keys found in the PEM given.
    ///
    /// Translates to DER format internally.
    pub fn from_pem(pem: &'a [u8]) -> Result<Self, Error> {
        let item = parse_pem(pem)
            .find(|p| matches!(p, Err(_) | Ok(PemItem::PrivateKey(_))))
            // None means there were no matches in the PEM chain
            .ok_or(Error::Certificate("no pem encoded private key found"))??;

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
        &self.der
    }

    /// Clones (allocates) to produce a static copy.
    pub fn to_owned(&self) -> PrivateKey<'static> {
        PrivateKey {
            kind: self.kind,
            der: Cow::Owned(self.der.to_vec()),
        }
    }
}

/// Parser of PEM data.
///
/// The data may contain one or many PEM items. The iterator produces the recognized PEM
/// items and skip others.
pub fn parse_pem(pem: &[u8]) -> impl Iterator<Item = Result<PemItem, Error>> + '_ {
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
    type Item = Result<PemItem<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match rustls_pemfile::read_one_from_slice(self.0) {
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
                    return Some(Err(Error::Pem(e)));
                }
            }
        }
    }
}

/// Load the root certificates from the system.
///
/// Used by [`TlsConfig::with_native_roots()`](super::TlsConfig::with_native_roots()). Exposed
/// as a helper for bespoke TLS implementations.
#[cfg(feature = "native-roots")]
pub fn load_native_root_certs() -> Vec<Certificate<'static>> {
    trace!("Try load root certs");
    let certs = match rustls_native_certs::load_native_certs() {
        Ok(v) => v,
        Err(e) => panic!("Failed to load root certs: {}", e),
    };

    let ret: Vec<_> = certs.into_iter().map(Certificate::from_pki_types).collect();

    debug!("Loaded {} root certs", ret.len());

    ret
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
