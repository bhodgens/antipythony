//! Cache layout (Plan 04 §Design):
//!
//! ```text
//! ~/.cache/antipythony/
//!   build/<hash>/          content-addressed build entries (04.02)
//!   site-packages/         uv --target fetch dir (04.01)
//! ```
//!
//! The `<hash>` input recipe (G-2: unpinned by the plan, documented here as
//! our choice): SHA-256 over the run input bytes, the lockfile bytes (empty
//! when absent), the toolchain version string, the backend tag, and the opt
//! level, joined with NUL separators. Entry dir holds the compiled artifact
//! (`run.py.bin` marker today; the artifact itself when 02.06 AoT lands) —
//! the tests only assert entry-set behaviour, never hash values.
//!
//! `ANTIPTYHONY_CACHE` overrides the root (documented in docs/packaging.md).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The cache root: `$ANTIPTYHONY_CACHE` or `$HOME/.cache/antipythony`.
pub fn cache_root() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("ANTIPTYHONY_CACHE") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    let home = std::env::var("HOME").ok()?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".cache").join("antipythony"))
}

/// The uv fetch target: `<root>/site-packages`.
pub fn site_packages_dir() -> Option<PathBuf> {
    cache_root().map(|r| r.join("site-packages"))
}

/// The build-cache dir: `<root>/build`.
pub fn build_cache_dir() -> Option<PathBuf> {
    cache_root().map(|r| r.join("build"))
}

/// Content-addressed key: SHA-256 over (input, lockfile, toolchain, backend,
/// opt level), hex-encoded. See the module docs for the recipe note.
pub fn build_key(
    input: &[u8],
    lockfile: &[u8],
    toolchain: &str,
    backend: &str,
    opt_level: u32,
) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input);
    h.update([0u8]);
    h.update(lockfile);
    h.update([0u8]);
    h.update(toolchain.as_bytes());
    h.update([0u8]);
    h.update(backend.as_bytes());
    h.update([0u8]);
    h.update(opt_level.to_le_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// A process-wide mutex serializing cache mutation (two concurrent `run`s
/// must not race an entry dir into existence twice). Never contended in the
/// test suite; cheap enough to take unconditionally.
pub fn cache_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    match LOCK.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Create (or reuse) the entry dir for `key`; returns its path.
pub fn entry_dir(key: &str) -> Option<PathBuf> {
    let dir = build_cache_dir()?.join(key);
    let _guard = cache_lock();
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Name of the future serialized-IR payload inside a build-cache entry
/// (B4, audit 2026-11-06-memperf-bughunt; CLI-side plumbing only — the
/// importer-side consumer + the `ap_ir::Module` serialization format are
/// decided in docs/compile-cache-design.md).
pub const COMPILED_PAYLOAD: &str = "compiled.bin";

/// Layout decision baked into the seam: the payload file embeds its cache
/// key as the first 64 bytes (hex of the SHA-256 `build_key`), then the
/// serialized IR body. Embedding (vs a sidecar file) makes the payload
/// self-validating: a truncated or re-keyed file can never be read as a
/// hit. Writers: see `write_compiled_payload`; readers: `compiled_hit`.
pub const COMPILED_KEY_LEN: usize = 64;

/// Atomically write a `compiled.bin` payload for `key` into entry dir
/// `dir` (temp file + rename under the cache lock). Nothing calls this
/// yet — it is the writer half of the seam that lands with the IR
/// serializer (02.06 AoT or the B4 importer-side consumer).
pub fn write_compiled_payload(dir: &Path, key: &str, body: &[u8]) -> std::io::Result<()> {
    let _guard = cache_lock();
    let tmp = dir.join(format!("{COMPILED_PAYLOAD}.tmp-{}", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(key.as_bytes())?; // must be the 64-hex build key
        f.write_all(body)?;
        f.flush()?;
    }
    std::fs::rename(&tmp, dir.join(COMPILED_PAYLOAD))?;
    Ok(())
}

/// Probe an entry dir for a key-matched `compiled.bin`. `Some(path)` = a
/// payload exists AND its embedded key equals `key` (a cache HIT for the
/// serialized-IR path); `None` = absent, unreadable, or key-mismatched
/// (caller must recompile and may overwrite). Never errors: a corrupt
/// payload is just a miss.
pub fn compiled_hit(dir: &Path, key: &str) -> Option<PathBuf> {
    let p = dir.join(COMPILED_PAYLOAD);
    let mut f = std::fs::File::open(&p).ok()?;
    let mut head = [0u8; COMPILED_KEY_LEN];
    use std::io::Read;
    f.read_exact(&mut head).ok()?;
    if head.as_slice() != key.as_bytes() {
        return None;
    }
    Some(p)
}

/// Remove the whole build cache (`cache gc`). Returns the number of entry
/// dirs removed. G-8: eviction policy is unspecced; v0 removes everything.
pub fn gc_build_cache() -> Result<usize, String> {
    let dir = build_cache_dir().ok_or_else(|| "no cache root (HOME unset?)".to_string())?;
    if !dir.exists() {
        return Ok(0);
    }
    let mut count = 0usize;
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            std::fs::remove_dir_all(&p)
                .map_err(|er| format!("cannot remove {}: {er}", p.display()))?;
            count += 1;
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_entry(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("apcache-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("cache.rs:174");
        dir
    }

    #[test]
    fn compiled_payload_roundtrip_and_key_match() {
        let dir = tmp_entry("rt");
        let key = build_key(b"src", b"", "0.0.0", "native", 2);
        // No payload yet -> miss.
        assert!(compiled_hit(&dir, &key).is_none());
        write_compiled_payload(&dir, &key, b"serialized-ir-bytes").expect("cache.rs:184");
        let hit = compiled_hit(&dir, &key).expect("key-matched payload is a hit");
        assert!(hit.ends_with(COMPILED_PAYLOAD));
        // A different key never matches the embedded header.
        let other = build_key(b"src2", b"", "0.0.0", "native", 2);
        assert!(compiled_hit(&dir, &other).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_payload_is_a_miss_not_a_panic() {
        let dir = tmp_entry("trunc");
        let key = build_key(b"s", b"", "0.0.0", "native", 2);
        std::fs::write(dir.join(COMPILED_PAYLOAD), &key.as_bytes()[..10]).expect("cache.rs:197");
        assert!(compiled_hit(&dir, &key).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
