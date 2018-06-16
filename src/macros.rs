
/// Create a `HashMap` from a shorthand notation.
///
/// ```
/// #[macro_use]
/// extern crate ureq;
///
/// fn main() {
/// let headers = map! {
///     "X-API-Key" => "foobar",
///     "Accept" => "application/json"
/// };
///
/// let agent = ureq::agent().set_map(headers).build();
/// }
/// ```
#[macro_export]
macro_rules! map(
    { $($key:expr => $value:expr),* } => {
        {
            let mut m = ::std::collections::HashMap::new();
            $(m.insert($key, $value);)+
            m
        }
     };
);
