#[allow(dead_code)]
mod macros;
mod serde_macros;
pub mod vecread;

use base64;
use mime_guess::get_mime_type_str;

pub use util::vecread::VecRead;

pub fn basic_auth(user: &str, pass: &str) -> String {
    let safe = match user.find(":") {
        Some(idx) => &user[..idx],
        None => user,
    };
    base64::encode(&format!("{}:{}", safe, pass))
}

pub fn mime_of<S: Into<String>>(s: S) -> String {
    let s = s.into();
    match &s[..] {
        "json" => "application/json",
        "form" => "application/x-www-form-urlencoded",
        _ => match get_mime_type_str(&s) {
            Some(mime) => mime,
            None => "foo",
        },
    }.to_string()
}
