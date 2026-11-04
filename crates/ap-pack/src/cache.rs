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

use std::path::PathBuf;
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
