# 2.0.0-rc2
 * Methods that formerly returned Response now return Result<Response, Error>.
   You'll need to change all instances of `.call()` to `.call()?` or handle
   errors using a `match` statement.
 * Non-2xx responses are considered Error by default. See [Error documentation]
   for details on how to get Response bodies for non-2xx.
 * Rewrite Error type. It's now an enum of two types of error: Status and
   Transport. Status errors (i.e. non-2xx) can be readily turned into a
   Response using match statements.
 * Errors now include the source error (e.g. errors from DNS or I/O) when
   appropriate, as well as the URL that caused an error.
 * The "synthetic error" concept is removed.
 * Move more configuration to Agent. Timeouts, TLS config, and proxy config
   now require building an Agent.
 * Create AgentBuilder to separate the process of building an agent from using
   the resulting agent. Headers can be set on an AgentBuilder, not the
   resulting Agent.
 * Agent is cheaply cloneable with an internal Arc. This makes it easy to
   share a single agent throughout your program.
 * There is now a default timeout_connect of 30 seconds. Read and write
   timeouts continue to be unset by default.
 * Add ureq::request_url and Agent::request_url, to send requests with
   already-parsed URLs.
 * Remove native_tls support.
 * Remove convenience methods `options(url)`, `trace(url)`, and `patch(url)`.
   To send requests with those verbs use `request(method, url)`.
 * Remove Request::build. This was a workaround because some of Request's
   methods took `&mut self` instead of `mut self`, and is no longer needed.
   You can simply delete any calls to `Request::build`.
 * Remove Agent::set_cookie.
 * Remove Header from the public API. The type wasn't used by any public
   methods.
 * Remove basic auth support. The API was incomplete. We may add back something
   better in the future.
 * Remove into_json_deserialize. Now into_json handles both serde_json::Value
   and other types that implement serde::Deserialize. If you were using
   serde_json before, you will probably have to explicitly annotate a type,
   like: `let v: serde_json::Value = response.into_json();`.
 * Rewrite README and top-level documentation.

[Error documentation]: https://docs.rs/ureq/2.0.0-rc4/ureq/enum.Error.html

# 2.0.0-rc4

 * Remove error_on_non_2xx. (#272)
 * Do more validation on status line. (#266)
 * (internal) Add history to response objects (#275)

# 2.0.0-rc3

 * Refactor Error to use an enum for easier extraction of status code errors.
 * (Internal) Use BufRead::read_line when reading headers.

# 2.0.0-rc2
 * These changes are mostly already listed under 2.0.0.
 * Remove the "synthetic error" concept. Methods that formerly returned
   Response now return Result<Response, Error>.
 * Rewrite Error type. Instead of an enum, it's now a struct with an
   ErrorKind. This allows us to store the source error when appropriate,
   as well as the URL that caused an error.
 * Move more configuration to Agent. Timeouts, TLS config, and proxy config
   now require building an Agent.
 * Create AgentBuilder to separate the process of building an agent from using
   the resulting agent. Headers can be set on an AgentBuilder, not the
   resulting Agent.
 * Agent is cheaply cloneable with an internal Arc. This makes it easy to
   share a single agent throughout your program.
 * There is now a default timeout_connect of 30 seconds. Read and write
   timeouts continue to be unset by default.
 * Add ureq::request_url and Agent::request_url, to send requests with
   already-parsed URLs.
 * Remove native_tls support.
 * Remove convenience methods `options(url)`, `trace(url)`, and `patch(url)`.
   To send requests with those verbs use `request(method, url)`.
 * Remove Request::build. This was a workaround because some of Request's
   methods took `&mut self` instead of `mut self`, and is no longer needed.
   You can simply delete any calls to `Request::build`.
 * Remove Agent::set_cookie.
 * Remove Header from the public API. The type wasn't used by any public
   methods.
 * Remove basic auth support. The API was incomplete. We may add back something
   better in the future.
 * Remove into_json_deserialize. Now into_json handles both serde_json::Value
   and other types that implement serde::Deserialize. If you were using
   serde_json before, you will probably have to explicitly annotate a type,
   like: `let v: serde_json::Value = response.into_json();`.
 * Rewrite README and top-level documentation.

# 1.5.2

 * Remove 'static constraint on Request.send(), allowing a wider variety of
   types to be passed. Also eliminate some copying. (#205).
 * Allow turning a Response into an Error (#214).
 * Update env_logger to 0.8.1 (#195).
 * Remove convenience method for CONNECT verb (#177).
 * Fix bugs in handling of timeout_read (#197 and #198).

# 1.5.1

 * Use cookie_store crate for correct cookie handling (#169).
 * Fix bug in picking wrong host for redirects introduced in 1.5.0 (#180).
 * Allow proxy settings on Agent (#178).

# 1.5.0

 * Add pluggable name resolution. Users can now override the IP addresses for
   hostnames of their choice (#148).
 * bugfix: Don't re-pool streams on drop. This would occur if the user called
   `response.into_reader()` and dropped the resulting `Read` before reading all
   the way to EOF. The result would be a BadStatus error on the next request to
   the same hostname. This only affected users using an explicit Agent (#160).
 * Automatically set Transfer-Encoding: chunked when using `send` (#86).
 * `into_reader()` now returns `impl Read + Send` instead of `impl Read` (#156).
 * Add support for log crate (#170).
 * Retry broken connections in more cases (should reduce BadStatus errors; #168).

# 1.4.1

 * Use buffer to avoid byte-by-byte parsing result in multiple syscalls.
 * Allow pooling multiple connections per host.
 * Put version in user agent "ureq/1.4.1".

# 1.4.0

  * Propagate WouldBlock in io:Error for Response::to_json.
  * Merge multiple cookies into one header to be spec compliant.
  * Allow setting TLSConnector for native-tls.
  * Remove brackets against TLS lib when IPv6 addr is used as hostname.
  * Include proxy in connection pool keys.
  * Stop assuming localhost for URLs without host part.
  * Error if body length is less than content-length.
  * Validate header names.

# 1.3.0

  * Changelog start
