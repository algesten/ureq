# ureq

![](https://github.com/algesten/ureq/workflows/CI/badge.svg)
[![CratesIO](https://img.shields.io/crates/v/ureq.svg)](https://crates.io/crates/ureq)
[![Documentation](https://docs.rs/ureq/badge.svg)](https://docs.rs/ureq)

> Minimal request library in rust.

## Usage

```rust
// sync post request of some json.
// requires feature:
// `ureq = { version = "*", features = ["json"] }`
let resp = ureq::post("https://myapi.example.com/ingest")
    .set("X-My-Header", "Secret")
    .send_json(serde_json::json!({
        "name": "martin",
        "rust": true
    }));

// .ok() tells if response is 200-299.
if resp.ok() {
  println!("success: {}", resp.into_string()?);
} else {
  // This can include errors like failure to parse URL or
  // connect timeout. They are treated as synthetic
  // HTTP-level error statuses.
  println!("error {}: {}", resp.status(), resp.into_string()?);
}
```

## About 1.0.0

This crate is now 1.x.x. It signifies there will be no more breaking
API changes (for better or worse). I personally use this code in
production system reading data from AWS. Whether the quality is good
enough for other use cases is a "YMMV".

## ureq's future

I asked for feedback on [ureq's future
direction](https://www.reddit.com/r/rust/comments/eu6qg8/future_of_ureq_http_client_library/)
and came to the conclusion that there's enough interest in a simple
blocking http client to keep it going. Another motivation is that I
use it extensively for my own work, to talk to S3.

I'll keep maintaining ureq. I will try to keep dependencies somewhat
fresh and try to address bad bugs. I will however not personally
implement new features in ureq, but I do welcome PR with open arms.

The code base is extremely simple, one might even call naive. It's a
good project to hack on as first learning experience in Rust. I will
uphold some base line of code hygiene, but won't block a PR due to
something being a bit inelegant.

## Features

To enable a minimal dependency tree, some features are off by default.
You can control them when including `ureq` as a dependency.

```
    ureq = { version = "*", features = ["json", "charset"] }
```

* `tls` enables https. This is enabled by default.
* `native-tls` enables https using the [`native-tls`](https://crates.io/crates/native-tls) crate. 
  NB: To make this work you currently need to use `default-features: false` to disable `tls`.
  We plan on fixing that.
* `json` enables `response.into_json()` and `request.send_json()` via serde_json.
* `charset` enables interpreting the charset part of
  `Content-Type: text/plain; charset=iso-8859-1`. Without this, the library
  defaults to rust's built in `utf-8`.

## Motivation

  * Minimal dependency tree
  * Obvious API
  * Blocking API
  * Convenience over correctness
  * No use of unsafe

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

### Sync forever

This library uses blocking socket reads and writes. When it was
created, there wasn't any async/await support in rust, and for my own
purposes, blocking IO was fine. At this point, one good reason to keep
this library going is that it is blocking (the other is that it does not
use unsafe).

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
