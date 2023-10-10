- Replace `impl From<http::request::Builder> for Request` with `TryFrom` because the conversion is fallible
  (implement in terms of `From<http::request::Parts>`: `builder.body(())?.into_parts().0.into()`);
