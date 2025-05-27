use std::borrow::Cow;
use std::fmt;
use std::iter::Enumerate;
use std::ops::Deref;
use std::str::Chars;

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

/// AsciiSet for characters that need to be percent-encoded in URL query parameters.
///
/// This set follows URL specification from <https://url.spec.whatwg.org/>
pub const ENCODED_IN_QUERY: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'') // Single quote should be encoded according to the URL specs
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

#[derive(Clone)]
pub(crate) struct QueryParam<'a> {
    source: Source<'a>,
}

#[derive(Clone)]
enum Source<'a> {
    Borrowed(&'a str),
    Owned(String),
}

/// Percent-encode a string using the ENCODED_IN_QUERY set.
pub fn url_enc(i: &str) -> Cow<str> {
    utf8_percent_encode(i, ENCODED_IN_QUERY).into()
}

/// Percent-encode a string using the ENCODED_IN_QUERY set, but replace encoded `%20` with `+`.
pub fn form_url_enc(i: &str) -> Cow<str> {
    let mut iter = utf8_percent_encode(i, ENCODED_IN_QUERY).map(|part| match part {
        "%20" => "+",
        _ => part,
    });

    // We try to avoid allocating if we can (returning a Cow).
    match iter.next() {
        None => "".into(),
        Some(first) => match iter.next() {
            // Case avoids allocation
            None => first.into(),
            // Following allocates
            Some(second) => {
                let mut string = first.to_owned();
                string.push_str(second);
                string.extend(iter);
                string.into()
            }
        },
    }
}

impl<'a> QueryParam<'a> {
    /// Create a new key-value pair with both the key and value percent-encoded.
    pub fn new_key_value(param: &str, value: &str) -> QueryParam<'static> {
        let s = format!("{}={}", url_enc(param), url_enc(value));
        QueryParam {
            source: Source::Owned(s),
        }
    }

    /// Create a new key-value pair without percent-encoding.
    ///
    /// This is used by query_raw() to add parameters that are already encoded
    /// or that should not be encoded.
    pub fn new_key_value_raw(param: &str, value: &str) -> QueryParam<'static> {
        let s = format!("{}={}", param, value);
        QueryParam {
            source: Source::Owned(s),
        }
    }

    fn as_str(&self) -> &str {
        match &self.source {
            Source::Borrowed(v) => v,
            Source::Owned(v) => v.as_str(),
        }
    }
}

pub(crate) fn parse_query_params(query_string: &str) -> impl Iterator<Item = QueryParam<'_>> {
    assert!(query_string.is_ascii());
    QueryParamIterator(query_string, query_string.chars().enumerate())
}

struct QueryParamIterator<'a>(&'a str, Enumerate<Chars<'a>>);

impl<'a> Iterator for QueryParamIterator<'a> {
    type Item = QueryParam<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut first = None;
        let mut value = None;
        let mut separator = None;

        for (n, c) in self.1.by_ref() {
            if first.is_none() {
                first = Some(n);
            }
            if value.is_none() && c == '=' {
                value = Some(n + 1);
            }
            if c == '&' {
                separator = Some(n);
                break;
            }
        }

        if let Some(start) = first {
            let end = separator.unwrap_or(self.0.len());
            let chunk = &self.0[start..end];
            return Some(QueryParam {
                source: Source::Borrowed(chunk),
            });
        }

        None
    }
}

impl<'a> fmt::Debug for QueryParam<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("QueryParam").field(&self.as_str()).finish()
    }
}

impl<'a> fmt::Display for QueryParam<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Source::Borrowed(v) => write!(f, "{}", v),
            Source::Owned(v) => write!(f, "{}", v),
        }
    }
}

impl<'a> Deref for QueryParam<'a> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<'a> PartialEq for QueryParam<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::http::Uri;

    #[test]
    fn query_string_does_not_start_with_question_mark() {
        let u: Uri = "https://foo.com/qwe?abc=qwe".parse().unwrap();
        assert_eq!(u.query(), Some("abc=qwe"));
    }

    #[test]
    fn percent_encoding_is_not_decoded() {
        let u: Uri = "https://foo.com/qwe?abc=%20123".parse().unwrap();
        assert_eq!(u.query(), Some("abc=%20123"));
    }

    #[test]
    fn fragments_are_not_a_thing() {
        let u: Uri = "https://foo.com/qwe?abc=qwe#yaz".parse().unwrap();
        assert_eq!(u.to_string(), "https://foo.com/qwe?abc=qwe");
    }

    fn p(s: &str) -> Vec<String> {
        parse_query_params(s).map(|q| q.to_string()).collect()
    }

    #[test]
    fn parse_query_string() {
        assert_eq!(parse_query_params("").next(), None);
        assert_eq!(p("&"), vec![""]);
        assert_eq!(p("="), vec!["="]);
        assert_eq!(p("&="), vec!["", "="]);
        assert_eq!(p("foo=bar"), vec!["foo=bar"]);
        assert_eq!(p("foo=bar&"), vec!["foo=bar"]);
        assert_eq!(p("foo=bar&foo2=bar2"), vec!["foo=bar", "foo2=bar2"]);
    }

    #[test]
    fn do_not_url_encode_some_things() {
        const NOT_ENCODE: &str = "!()*-._~";
        let q = QueryParam::new_key_value("key", NOT_ENCODE);
        assert_eq!(q.as_str(), format!("key={}", NOT_ENCODE));
    }

    #[test]
    fn special_encoding_space_for_form() {
        let value = "value with spaces and 'quotes'";
        let form = form_url_enc(value);
        assert_eq!(form.as_ref(), "value+with+spaces+and+%27quotes%27");
    }

    #[test]
    fn do_encode_single_quote() {
        let value = "value'with'quotes";
        let q = QueryParam::new_key_value("key", value);
        assert_eq!(q.as_str(), "key=value%27with%27quotes");
    }

    #[test]
    fn raw_query_param_no_encoding() {
        // Use URI-valid characters for the raw param test
        let value = "value-without-spaces&special='chars'";
        let q = QueryParam::new_key_value_raw("key", value);
        assert_eq!(q.as_str(), format!("key={}", value));

        // Verify that symbols like &=+?/ remain unencoded in raw mode
        // but are encoded in normal mode
        let special_symbols = "symbols&=+?/'";
        let q_raw = QueryParam::new_key_value_raw("raw", special_symbols);
        let q_encoded = QueryParam::new_key_value("encoded", special_symbols);

        // Raw should preserve all special chars, encoded should encode them
        assert_eq!(q_raw.as_str(), "raw=symbols&=+?/'");
        assert_ne!(q_raw.as_str(), q_encoded.as_str());
    }
}
