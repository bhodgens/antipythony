//! Packaging subsystem (leaves 04.01 + 04.02).
//!
//! Layout:
//! - [`manifest`]: `antipythany.toml` (project deps) + `antipythany.lock`
//!   (pinned versions, uv provenance) — the T-14 G-1 derived schema.
//! - [`uv`]: the uv subprocess wrapper. uv is standalone Apache-2.0 and is
//!   used ONLY as a subprocess, never vendored/linked (Plan 04 §Design).
//! - [`purity`]: the pure-wheel-only acceptance filter + the C-08/C-10
//!   native-component diagnostic, applied at ADD time (fail early).
//! - [`cache`]: content-addressed build cache under
//!   `~/.cache/antipythany/build/<hash>` (Plan 04 §Design "Build cache") and
//!   the site-packages dir `~/.cache/antipythany/site-packages` the fetched
//!   wheels land in. `ANTPYTHANY_CACHE` overrides the cache root.
//! - [`project`]: `new` scaffold + cwd/upward manifest discovery.

pub mod cache;
pub mod manifest;
pub mod project;
pub mod purity;
pub mod uv;
