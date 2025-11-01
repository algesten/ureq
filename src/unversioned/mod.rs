//! API that does not (yet) follow semver.
//!
//! All public types under `unversioned` are available to use, but are not considered final
//! API in the semver sense. Breaking changes to anything under the module `unversioned`,
//! like `Transport` or `Resolver` will NOT be reflected in a major version bump of the
//! `ureq` crate. We do however commit to only make such changes in *minor* version bumps,
//! not patch.
//!
//! In time, we will move these types out of `unversioned` and solidify the API. There
//! is no set timeline for this.

pub mod resolver;
pub mod transport;

#[cfg(feature = "multipart")]
#[path = "../multipart.rs"]
pub mod multipart;
