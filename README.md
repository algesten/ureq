# ureq

![](https://github.com/algesten/ureq/workflows/CI/badge.svg)
[![CratesIO](https://img.shields.io/crates/v/ureq.svg)](https://crates.io/crates/ureq)
[![Documentation](https://docs.rs/ureq/badge.svg)](https://docs.rs/ureq)


> Minimal request library in rust.

## Usage

```rust
// requires feature: `ureq = { version = "*", features = ["json"] }`
#[macro_use]
extern crate ureq;

fn main() {

    // sync post request of some json.
    let resp = ureq::post("https://myapi.acme.com/ingest")
        .set("X-My-Header", "Secret")
        .send_json(json!({
            "name": "martin",
            "rust": true
        }));

    // .ok() tells if response is 200-299.
    if resp.ok() {
        // ...
    }
}
```

## Features

To enable a minimal dependency tree, some features are off by default.
You can control them when including `ureq` as a dependency.

```
    ureq = { version = "*", features = ["json", "charset"] }
```

* `tls` enables https. This is enabled by default.
* `json` enables `response.into_json()` and `request.send_json()` serde json.
* `charset` enables interpreting the charset part of
  `Content-Type: text/plain; charset=iso-8859-1`. Without this, the library
  defaults to rust's built in `utf-8`.

## Motivation

  * Minimal dependency tree
  * Obvious API
  * Convencience over correctness

This library tries to provide a convenient request library with a minimal dependency
tree and an obvious API. It is inspired by libraries like
[superagent](http://visionmedia.github.io/superagent/) and
[fetch API](https://developer.mozilla.org/en-US/docs/Web/API/Fetch_API).

This library does not try to enforce web standards correctness. It uses HTTP/1.1,
but whether the request is _perfect_ HTTP/1.1 compatible is up to the user of the
library. For example:

```rust
    let resp = ureq::post("https://myapi.acme.com/blah")
        .set("Jättegött", "Vegankörv")
        .call();
```

The header name and value would be encoded in utf-8 and sent, but that is actually not
correct according to spec cause an HTTP header name should be ascii. The absolutely
correct way would be to have `.set(header, value)` return a `Result`. This library opts
for convenience over correctness, so the decision is left to the user.

### Sync (for now)

This library uses blocking socket reads and writes, for now.
The async story in rust is in heavy development and when used
currently pulls in a heavy dependency tree (tokio etc). Once
more async support is in rust core and won't drag those
dependencies, this library might change.


## TODO

- [ ] Forms with application/x-www-form-urlencoded
- [ ] multipart/form-data
- [ ] Expect 100-continue
- [x] Use `rustls` when [ring with versioned asm symbols](https://github.com/briansmith/ring/pull/619) is released. (PR is not resolved, but most implementations have settled on 0.13)

## License

Copyright (c) 2019 Martin Algesten

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
