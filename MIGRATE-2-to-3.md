# Changes ureq 2.x -> 3.x

This is not an exhaustive list of changes. Most tweaks to the API are clear by looking
at the docs. If anything is unclear, please open a PR and we can clarify further.

## HTTP Crate

In 2.x ureq implemented it's own `Request` and `Response` structs. In 3.x, we
drop our own impl in favor of the [http crate]. The http crate presents a unified HTTP
API and can be found as a dependency of a number of big [http-related crates] in the
Rust ecosystem. The idea is that presenting a well-known API towards users of ureq
will make it easier to use.

* .set -> .header

```
// ureq2.x
ureq::get("https://httpbin.org/get").set("foo", "bar").call();
// ureq3.x
ureq::get("https://httpbin.org/get").header("foo", "bar").call();
```

## Re-exported crates must be semver 1.x (stable)

ureq2.x re-exported a number of semver 0.x crates and thus suffered from that breaking
changes in those crates technically were breaking changes in ureq (and thus ought to increase
major version). In ureq 3.x we will strive to re-export as few crates as possible.

* No re-exported tls config
* No re-exported cookie crates
* No re-exported json macro

Instead we made our own TLS config and Cookie API, and drop the json macro.



## No retry idempotent

ureq 2.x did an automatic retry of idempotent methods (GET, HEAD). This was considered
confusing, so 3.x has no built-in retries.

## No send body charset

For now, ureq 3.x can't change the charset of a send body. It can however still do that
for the response body.

[http crate]: https://crates.io/crates/http
[http-related crates]: https://crates.io/crates/http/reverse_dependencies
