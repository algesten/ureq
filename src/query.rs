use core::{iter::Enumerate, ops::Deref, str::Chars};

use alloc::{borrow::Cow, fmt, string::String};
use percent_encoding::utf8_percent_encode;

#[derive(Clone)]
pub(crate) struct QueryParam<'a> {
    source: Source<'a>,
}

#[derive(Clone)]
enum Source<'a> {
    Borrowed(&'a str),
    Owned(String),
}

pub fn url_enc(i: &str) -> Cow<str> {
    utf8_percent_encode(i, percent_encoding::NON_ALPHANUMERIC).into()
}

impl<'a> QueryParam<'a> {
    pub fn new_key_value(param: &str, value: &str) -> QueryParam<'static> {
        let s = alloc::format!("{}={}", url_enc(param), url_enc(value));
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
    use alloc::vec::Vec;

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
        assert_eq!(p("&"), alloc::vec![""]);
        assert_eq!(p("="), alloc::vec!["="]);
        assert_eq!(p("&="), alloc::vec!["", "="]);
        assert_eq!(p("foo=bar"), alloc::vec!["foo=bar"]);
        assert_eq!(p("foo=bar&"), alloc::vec!["foo=bar"]);
        assert_eq!(p("foo=bar&foo2=bar2"), alloc::vec!["foo=bar", "foo2=bar2"]);
    }
}
