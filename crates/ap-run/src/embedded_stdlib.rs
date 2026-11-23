//! The embedded vendored_stdlib source store (leaf tier1-linecache).
//!
//! 02.06 follow-up / 02.08 audit remaining-work row 4: an AoT binary ships
//! WITHOUT the build machine's vendored_stdlib/ directory, but its import
//! chains (enum/warnings/inspect/traceback/contextlib/...) and the
//! linecache/traceback source consumers still need the vendored pure-Python
//! sources. This module is the READ side of a static table generated from
//! the vendored tree at build time (xtask/gen_embedded_stdlib.sh →
//! embedded_stdlib_data.rs); the native registry stays the FIRST resolution
//! face — the embedded store only replaces the file finder when the
//! vendored directory is ABSENT from the filesystem (the standalone-binary
//! case). With the directory present (repo runs, `antipythony run`), every
//! lookup behaves exactly as before: disk bytes win, zero divergence.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::collections::HashMap;
use std::sync::OnceLock;

mod data;
pub use data::EMBEDDED_STDLIB_FILE_COUNT;

/// The vendored dir path RECORDED AT BUILD TIME (CARGO_MANIFEST_DIR of
/// ap-run → the workspace root). At runtime the importer compares its
/// discovered vendored dir against this: a DIFFERENT (or absent) tree means
/// the binary is running standalone and the embedded store arms engage.
pub fn build_time_vendored_dir() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendored_stdlib")
}

/// Map a module name to its vendored source bytes, or None for names the
/// generated store doesn't carry.
///
/// Lookup conventions (the importer's file-finder shapes):
/// - "enum" → vendored_stdlib/enum.py;
/// - "encodings.aliases" → vendored_stdlib/encodings/aliases.py;
/// - "collections.abc" has NO own file in the vendored tree (the importer
///   synthesizes it from _collections_abc.py) — None, as on disk;
/// - "package.__init__" style is not used: package keys are the bare
///   package name ("encodings" → encodings/__init__.py).
pub fn source_for(name: &str) -> Option<&'static [u8]> {
    // The flat index is tiny (~470 rows); linear scan from a binary edge is
    // negligible next to the parse that always follows. No lazy HashMap.
    data::EMBEDDED_STDLIB_SOURCES
        .binary_search_by_key(&name, |&(k, _)| k)
        .ok()
        .map(|i| data::EMBEDDED_STDLIB_SOURCES[i].1)
}

/// All keys (import paths) — used by tests + the diagnostics surface.
pub fn keys() -> impl Iterator<Item = &'static str> {
    data::EMBEDDED_STDLIB_SOURCES.iter().map(|(k, _)| *k)
}

/// A decoded source for `name` (lossy UTF-8 — the vendored tree is ASCII/
/// UTF-8; a decode error would be a vendored-tree bug, not a runtime one).
pub fn source_string_for(name: &str) -> Option<String> {
    source_for(name).map(|b| String::from_utf8_lossy(b).into_owned())
}

/// Store sanity: sorted + non-empty (a broken regen would silently shadow
/// nothing but also serve nothing — fail the test suite instead).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_is_sorted_and_nonempty() {
        let keys: Vec<&str> = keys().collect();
        assert!(keys.len() > 300, "store regen lost files: {}", keys.len());
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "store must stay sorted for binary_search");
        for k in &keys {
            assert!(!source_for(k).unwrap_or(&[]).is_empty(), "empty: {k}");
        }
    }

    #[test]
    fn known_faces_resolve() {
        for k in ["enum", "warnings", "inspect", "traceback", "contextlib",
                  "encodings", "encodings.aliases", "unittest"] {
            assert!(source_for(k).is_some(), "missing embedded face: {k}");
        }
        // The importer synthesizes collections.abc from _collections_abc —
        // the store mirrors the DISK (no collections/abc.py), so None.
        assert!(source_for("collections.abc").is_none());
    }
}
