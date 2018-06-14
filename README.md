# ureq

> Minimal request library in rust.

# UNDER CONSTRUCTION

- [x] Somewhat finished API
- [x] TLS
- [x] Header handling
- [x] Transfer-Encoding: chunked
- [x] Ergonomic JSON handling
- [x] Test harness for end-to-end tests
- [x] Always chunked RFC2616 #3.6
- [x] Limit read length on Content-Size
- [x] Auth headers
- [x] Repeated headers
- [x] Cookie jar in agent
- [ ] Forms with application/x-www-form-urlencoded
- [ ] multipart/form-data
- [ ] Connection reuse/keep-alive with pool
- [ ] Expect 100-continue
- [ ] Use `rustls` when ring v0.13 is released.

## Usage

```rust
#[macro_use]
extern create ureq;

// sync post request of some json.
let resp = ureq::post("https://myapi.acme.com/ingest")
    .set("X-My-Header", "Secret")
    .send_json(json!({
        "name": "martin",
        "rust": true
    }));

// .ok() tells if response is 200-299.
assert!(resp.unwrap().ok());
```

## Motivation

  * Minimal dependency tree
  * Obvious API

This library tries to provide a convenient request library with a minimal dependency
tree and an obvious API. It is inspired by libraries like
[superagent](http://visionmedia.github.io/superagent/) and
[fetch API](https://developer.mozilla.org/en-US/docs/Web/API/Fetch_API).

### Sync (for now)

This library uses blocking socket reads and writes, for now.
The async story in rust is in heavy development and when used
currently pulls in a heavy dependency tree (tokio etc). Once
more async support is in rust core and won't drag those
dependencies, this library might change.

## License (MIT)

Copyright (c) 2018 Martin Algesten

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
