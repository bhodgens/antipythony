//! `antipythony` — the driver binary (Plan 04 CLI surface).
//!
//! Verbs (04.01 + 04.02):
//! ```text
//! antipythony new <name>            # scaffold: <name>.py + antipythony.toml
//! antipythony add <pkg|file:path>   # uv fetch (pure wheels only) + lock
//! antipythony run <file.py>         # tier-0 execute (deps on sys.path)
//! antipythony run --no-cache ...    # bypass the content-addressed cache
//! antipythony build <file.py> [-o out]  # (04.02) cached compile artifact
//! antipythony cache gc              # clear the build cache
//! ```
//!
//! `--tier 0` is the only execution mode (this binary IS tier-0; other modes
//! are rejected loudly so adaptive/binary gates cannot report green on
//! tier-0 output).

use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    // 2026-11-06 OOM mitigation: in-process RSS self-guard on EVERY
    // interpreter process, so an unbounded-growth runtime bug aborts the
    // process instead of OOMing the machine (macOS has no OOM killer).
    // AP_MAX_RSS_GB overrides; default = 25% of physical RAM.
    ap_run::rss_guard::start();
    let args: Vec<String> = std::env::args().collect();
    let code = match run(&args) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    };
    std::process::exit(code);
}

fn usage() -> i32 {
    eprintln!(
        "usage: antipythony <verb> [args]\n\
         \n\
         verbs:\n\
         \x20 new <name>                  scaffold <name>/ with antipythony.toml + <name>.py\n\
         \x20 add <pkg>                   fetch a pure-Python package via uv + lock it\n\
         \x20 run [--no-cache] <file.py>  execute (fetched deps join sys.path)\n\
         \x20 build <file.py> [-o out]    write the cached build artifact\n\
         \x20 cache gc                    clear the build cache"
    );
    2
}

fn run(args: &[String]) -> Result<i32, String> {
    let mut i = 1;
    // Pre-scan for --tier (accepted on every verb, forwarded to execution).
    let mut tier: Option<String> = None;
    while i < args.len() {
        match args[i].as_str() {
            "--tier" => {
                tier = Some(
                    args.get(i + 1)
                        .ok_or("antipythony: --tier requires a mode (0 | adaptive)")?
                        .clone(),
                );
                i += 2;
            }
            other if other.starts_with("--tier=") => {
                tier = Some(other["--tier=".len()..].to_string());
                i += 1;
            }
            _ => break,
        }
    }
    if let Some(mode) = &tier {
        if mode != "0" {
            return Err(format!(
                "antipythony: --tier {mode} is not implemented; this binary runs tier-0 only \
                 (pass --tier 0 or omit --tier)"
            ));
        }
    }

    let verb = args.get(i).map(|s| s.as_str());
    let rest: Vec<&str> = args[i + 1..].iter().map(|s| s.as_str()).collect();
    match verb {
        None => Ok(usage()),
        Some("new") => cmd_new(&rest),
        Some("add") => cmd_add(&rest),
        Some("run") => cmd_run(&rest, false),
        Some("build") => cmd_build(&rest),
        Some("cache") => cmd_cache(&rest),
        // Single-file legacy form: `antipythony file.py` (02.02 seam).
        Some(other) if other.ends_with(".py") => {
            let r2: Vec<&str> = vec![other];
            cmd_run(&r2, false)
        }
        Some(other) if other != "run" && rest.iter().any(|a| a.ends_with(".py")) => {
            // `--tier 0 file.py` fell through the pre-scan into verb position.
            let r2: Vec<&str> = std::iter::once(other).chain(rest.iter().copied()).collect();
            cmd_run(&r2, false)
        }
        Some(other) => {
            eprintln!("antipythony: unexpected argument '{other}'");
            Ok(usage())
        }
    }
}

// ---------------------------------------------------------------------------
// new
// ---------------------------------------------------------------------------

fn cmd_new(rest: &[&str]) -> Result<i32, String> {
    let name = rest.first().ok_or("usage: antipythony new <name>")?;
    let dir = ap_pack::project::new_project(Path::new("."), name)?;
    println!("created {} (antipythony.toml, {name}.py)", dir.display());
    Ok(0)
}

// ---------------------------------------------------------------------------
// add — the 04.01 uv fetch + lock
// ---------------------------------------------------------------------------

fn cmd_add(rest: &[&str]) -> Result<i32, String> {
    // `--no-deps` form: `antipythony add --no-deps "<req> <req> ..."` installs
    // exactly the named requirements (space-separated uv specs) without
    // transitive resolution. Escape hatch for ecosystems whose uv closure
    // contains platform wheels antipythony must refuse (e.g. scrapy ->
    // lxml/brotli/cryptography): the caller pins the pure subset. The purity
    // filter still runs — anything C-flavored still fails loud (C-11).
    let (no_deps, rest) = match rest.first() {
        Some(&"--no-deps") => (true, &rest[1..]),
        _ => (false, rest),
    };
    let spec = rest
        .first()
        .ok_or("usage: antipythony add [--no-deps] <package | req-list | file:<path | .whl>>")?;
    // File-path argument forms: a bare path to a wheel/dir, or `file:<path>`
    // (G-6: the offline form the test track assumes; uv's own convention).
    let uv_spec: String = normalize_spec(spec);

    // Project dir: cwd must be one (or inside one). `new` creates it.
    let proj = ap_pack::manifest::find_project_dir(Path::new("."))
        .ok_or("antipythony: no antipythony.toml here (run `antipythony new <name>` first)")?;
    let mut manifest = ap_pack::manifest::read_manifest(&proj)?.unwrap_or_default();

    // uv version probe FIRST so a uv-absent environment fails with the
    // helpful error before anything is fetched (T-14 missing-uv test).
    let version = ap_pack::uv::uv_version()?;

    // Stage into a temp dir INSIDE the cache root (same volume => atomic
    // rename), inspect for native components, promote only if pure.
    let staging = staging_dir()?;
    let staged = if no_deps {
        ap_pack::uv::install_staged_no_deps(&uv_spec, &staging)?
    } else {
        ap_pack::uv::install_staged(&uv_spec, &staging)?
    };
    if let Err(diag) = ap_pack::purity::check_pure(&staged.dir) {
        // C-11: total refusal — nothing enters the cache, lock, or manifest.
        let _ = std::fs::remove_dir_all(&staged.dir);
        return Err(diag);
    }

    // Promote into the live site-packages.
    let Some(sp) = ap_pack::cache::site_packages_dir() else {
        return Err("antipythony: no cache root (HOME unset?)".to_string());
    };
    std::fs::create_dir_all(&sp)
        .map_err(|e| format!("antipythony: cannot create {}: {e}", sp.display()))?;
    // Merge staging contents into site-packages (uv itself is idempotent on
    // re-add; the merge keeps multiple adds of different packages working).
    merge_dirs(&staged.dir, &sp)?;
    let _ = std::fs::remove_dir_all(&staged.dir);

    // Record: manifest [[dependency]] + lock [[package]] pins.
    let dep_name = display_name(spec);
    if !manifest.dependencies.iter().any(|d| d.name == dep_name) {
        manifest.dependencies.push(ap_pack::manifest::Dep {
            name: dep_name.clone(),
            source: "file-cache".to_string(),
        });
    }
    ap_pack::manifest::write_manifest(&proj, &manifest)?;

    let mut lock = ap_pack::manifest::read_lock(&proj)?.unwrap_or(ap_pack::manifest::Lockfile {
        lock_version: 1,
        created_by: "uv".to_string(),
        packages: Vec::new(),
    });
    for (name, ver) in &staged.installed {
        match lock.packages.iter_mut().find(|p| &p.name == name) {
            Some(p) => p.version = ver.clone(),
            None => lock.packages.push(ap_pack::manifest::LockedPkg {
                name: name.clone(),
                version: ver.clone(),
            }),
        }
    }
    ap_pack::manifest::write_lock(&proj, &lock)?;

    println!(
        "added {dep_name} ({})",
        staged
            .installed
            .iter()
            .map(|(n, v)| format!("{n}=={v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("resolved with {}", version);
    Ok(0)
}

/// `file:/abs/path`, `file:rel`, bare `/abs`, bare `./rel`, bare `.whl` and
/// bare package names all normalize to a uv requirement spec.
fn normalize_spec(spec: &str) -> String {
    if let Some(p) = spec.strip_prefix("file:") {
        return p.to_string();
    }
    // Bare name w/o path separators or extension: registry spec, as-is.
    if !spec.contains('/') && !spec.ends_with(".whl") && !spec.starts_with('.') {
        return spec.to_string();
    }
    spec.to_string()
}

fn display_name(spec: &str) -> String {
    let s = spec.strip_prefix("file:").unwrap_or(spec);
    let base = Path::new(s)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| s.to_string());
    match base.rfind('-') {
        // wheel filename {name}-{ver}-...whl -> name part
        Some(idx) if base.ends_with(".whl") => base[..idx].to_string(),
        _ => {
            if let Some(stripped) = base.strip_suffix(".whl") {
                stripped.split('-').next().unwrap_or(stripped).to_string()
            } else {
                base
            }
        }
    }
}

/// Staging dir: <cache>/staging-<pid>-<nanos> (unique per add, same volume).
fn staging_dir() -> Result<PathBuf, String> {
    let root = ap_pack::cache::cache_root().ok_or("antipythony: no cache root (HOME unset?)")?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(root.join(format!("staging-{}-{nanos}", std::process::id())))
}

/// Promote staging contents into `dst` by MOVING entries (F3, audit
/// 2026-11-06-memperf-bughunt): staging lives inside the cache root (same
/// volume — that's the whole reason `staging_dir()` puts it there), so each
/// top-level entry is an atomic `rename`, not a full-tree copy. The old
/// recursive `std::fs::copy` double-copied every wheel we ever added.
///
/// Semantics preserved exactly (the merged file set + bytes are identical
/// to the old copy-merge):
/// * destination absent → whole entry renamed (dirs keep their tree shape);
/// * collision of two dirs (re-add of the same package: `six/`,
///   `six-*.dist-info/`) → recursive merge, same as before;
/// * file over existing file → overwrite (`rename` replaces atomically,
///   where `copy` overwrote byte-wise — same end state);
/// * EXDEV (cross-volume, e.g. ANTIPTYHONY_CACHE on another fs) → fall back
///   to the old recursive copy for that entry.
fn merge_dirs(src: &Path, dst: &Path) -> Result<(), String> {
    // Entry-not-exists fast path first, then collide-merge below.
    if !dst.exists() {
        match std::fs::rename(src, dst) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                return copy_tree(src, dst);
            }
            Err(_) if dst.exists() && src.is_dir() && dst.is_dir() => {
                // Lost a race with a concurrent add that created dst;
                // fall through to the merge path below.
            }
            Err(e) => return Err(format!("cannot move {}: {e}", src.display())),
        }
    }
    if src.is_dir() && dst.is_dir() {
        for e in std::fs::read_dir(src)
            .map_err(|e| format!("cannot read {}: {e}", src.display()))?
            .flatten()
        {
            merge_dirs(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else if !src.is_dir() {
        match std::fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                std::fs::copy(src, dst)
                    .map_err(|e| format!("cannot copy {}: {e}", src.display()))?;
                Ok(())
            }
            Err(e) => Err(format!("cannot move {}: {e}", src.display())),
        }
    } else {
        // dir onto an existing file: the old code failed here too
        // (create_dir_all on a file path). Keep failing loudly.
        Err(format!(
            "cannot merge dir {} onto non-dir {}",
            src.display(),
            dst.display()
        ))
    }
}

/// EXDEV fallback: the pre-F3 recursive copy-merge, byte-for-byte behavior
/// retained for cross-volume staging/cache roots.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    for e in std::fs::read_dir(src)
        .map_err(|e| format!("cannot read {}: {e}", src.display()))?
        .flatten()
    {
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|e| format!("cannot copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

fn cmd_run(rest: &[&str], _wasm: bool) -> Result<i32, String> {
    let mut path: Option<String> = None;
    let mut no_cache = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--no-cache" => no_cache = true,
            "--wasm" => {
                return Err(
                    "antipythony: --wasm is not implemented (Plan 03 / leaf 04.04 territory)"
                        .to_string(),
                )
            }
            other if path.is_none() => path = Some(other.to_string()),
            other => return Err(format!("antipythony: unexpected argument '{other}'")),
        }
        i += 1;
    }
    let Some(path) = path else {
        return Err("usage: antipythony run [--no-cache] <file.py>".to_string());
    };
    let source = std::fs::read_to_string(&path)
        .map_err(|e| format!("antipythony: cannot read {path}: {e}"))?;

    // Content-addressed cache (04.02): key = (input bytes, lockfile bytes,
    // toolchain, backend, opt level). `--no-cache` bypasses both read and
    // write. The artifact is the compiled-module marker today (tier-0
    // re-executes the source; 02.06 AoT replaces the payload).
    //
    // B4 (audit 2026-11-06-memperf-bughunt): the cache entry dir is now
    // probed for a `compiled.bin` payload — a future serialized
    // `ap_ir::Module` + `ScopeInfoModule` (see docs/compile-cache-design.md
    // for the format decision). When present and key-matched, tier-0 can
    // skip parse+lower; until that importer-side consumer lands, a
    // key-matched hit is reported and we still fall through to re-parse
    // (the seam, not yet a behavior change — byte-exact output is
    // unchanged either way).
    let script_dir = Path::new(&path)
        .parent()
        .map(|p| {
            // `app.py` yields parent "" — absolutize so upward manifest
            // discovery walks the real directories.
            if p.as_os_str().is_empty() {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            } else {
                p.to_path_buf()
            }
        })
        .unwrap_or_else(|| PathBuf::from("."));
    let lock_bytes = ap_pack::manifest::find_project_dir(&script_dir)
        .and_then(|d| std::fs::read(d.join(ap_pack::manifest::LOCK_FILE)).ok())
        .unwrap_or_default();
    let key = ap_pack::cache::build_key(
        source.as_bytes(),
        &lock_bytes,
        env!("CARGO_PKG_VERSION"),
        "native",
        2,
    );
    let mut cache_entry: Option<PathBuf> = None;
    if !no_cache {
        if let Some(dir) = ap_pack::cache::entry_dir(&key) {
            // B4 read-back seam: a `compiled.bin` payload whose embedded
            // key matches is a serialized-IR HIT. The importer-side
            // consumer (deserialize -> ap_ir::Module + ScopeInfoModule,
            // skip parse+lower) is the other half of B4 and is NOT in this
            // file's ownership; until it lands we probe, surface the hit
            // under AP_DEBUG_CACHE, and fall through to tier-0 re-parse so
            // behavior and output bytes are unchanged.
            if let Some(payload) = ap_pack::cache::compiled_hit(&dir, &key) {
                if std::env::var_os("AP_DEBUG_CACHE").is_some() {
                    eprintln!(
                        "antipythony: compile-cache hit {} (payload not yet consumed — \
                         see docs/compile-cache-design.md)",
                        payload.display()
                    );
                }
            }
            let marker = dir.join("artifact");
            if marker.exists() {
                cache_entry = Some(marker);
            } else {
                let _ = std::fs::write(&marker, source.as_bytes());
                cache_entry = Some(marker);
            }
        }
    }
    // `cache_entry` keeps the entry dir alive for gc accounting and marks
    // the key as populated; execution below is tier-0 re-parse (B4 seam).
    let _ = cache_entry;

    // The tier-0 interpreter consumes ~50KB of native stack per Python frame
    // (recursion_limit 1000 needs ~50MB): run the module on a dedicated
    // thread with a 256MB stack (same contract as ap-run-eval).
    let script_path = path.clone();
    let exit_code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let mut out = std::io::stdout();
            let mut err = std::io::stderr();
            let outcome = ap_run::run_module(&source, &script_path, &mut out, &mut err);
            let _ = out.flush();
            let _ = err.flush();
            outcome.exit_code
        })
        .map_err(|e| format!("antipythony: spawn interpreter thread: {e}"))?
        .join()
        .unwrap_or(134); // match a crash exit if the thread panics
    Ok(exit_code)
}

// ---------------------------------------------------------------------------
// build
// ---------------------------------------------------------------------------

fn cmd_build(rest: &[&str]) -> Result<i32, String> {
    let mut path: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "-o" => {
                out = Some(PathBuf::from(
                    rest.get(i + 1)
                        .ok_or("antipythony: -o requires a path")?
                        .to_string(),
                ));
                i += 2;
                continue;
            }
            other => {
                if path.is_some() {
                    return Err(format!("antipythony: unexpected argument '{other}'"));
                }
                path = Some(other.to_string());
            }
        }
        i += 1;
    }
    let Some(path) = path else {
        return Err("usage: antipythony build <file.py> [-o out]".to_string());
    };
    let source = std::fs::read_to_string(&path)
        .map_err(|e| format!("antipythony: cannot read {path}: {e}"))?;
    let script_dir = Path::new(&path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let lock_bytes = ap_pack::manifest::find_project_dir(&script_dir)
        .and_then(|d| std::fs::read(d.join(ap_pack::manifest::LOCK_FILE)).ok())
        .unwrap_or_default();
    let key = ap_pack::cache::build_key(
        source.as_bytes(),
        &lock_bytes,
        env!("CARGO_PKG_VERSION"),
        "native",
        2,
    );
    let out_path = out.unwrap_or_else(|| {
        Path::new(&path)
            .file_stem()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("app"))
    });
    if let Some(dir) = ap_pack::cache::entry_dir(&key) {
        // Content-addressed staging: identical input -> identical artifact
        // bytes (04.02 reproducibility). The tier-0 artifact is the source
        // marker; the OUTPUT is a self-contained runner script pointing at
        // this binary — byte-identical across clean caches by construction.
        let marker = dir.join("artifact");
        let _ = std::fs::write(&marker, source.as_bytes());
        // B4 seam (mirrors cmd_run): `compiled.bin` + matching embedded key
        // = serialized-IR hit. Writer half (`ap_pack::cache::
        // write_compiled_payload`) fires when the IR serializer lands;
        // until then a hit is surfaced under AP_DEBUG_CACHE and build
        // proceeds with the marker payload (behavior unchanged).
        if let Some(payload) = ap_pack::cache::compiled_hit(&dir, &key) {
            if std::env::var_os("AP_DEBUG_CACHE").is_some() {
                eprintln!(
                    "antipythony: build compile-cache hit {} (payload not yet consumed — \
                     see docs/compile-cache-design.md)",
                    payload.display()
                );
            }
        }
    }
    // Emit a deterministic launcher (no timestamps, no absolute paths except
    // the interpreter the launcher execs — resolved at run time).
    std::fs::write(
        &out_path,
        "#!/bin/sh\nexec antipythony run \"$0.py\" \"$@\"\n",
    )
    .map_err(|e| format!("antipythony: cannot write {}: {e}", out_path.display()))?;
    println!("built {}", out_path.display());
    Ok(0)
}

// ---------------------------------------------------------------------------
// cache
// ---------------------------------------------------------------------------

fn cmd_cache(rest: &[&str]) -> Result<i32, String> {
    match rest {
        ["gc"] => {
            let n = ap_pack::cache::gc_build_cache()?;
            println!("cache gc: removed {n} build entries");
            Ok(0)
        }
        _ => Err("usage: antipythony cache gc".to_string()),
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    /// Snapshot of a tree: relative path -> bytes, so the rename-merge and
    /// the old copy-merge can be compared for identical file SET + CONTENTS
    /// (the F3 contract: same-volume promotion must not change what lands
    /// in site-packages).
    fn snapshot(root: &Path, cur: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(cur).expect("main.rs:559").flatten() {
            let p = e.path();
            if p.is_dir() {
                snapshot(root, &p, out);
            } else {
                let rel = p
                    .strip_prefix(root)
                    .expect("main.rs:564")
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read(&p).expect("main.rs:565")));
            }
        }
    }

    fn snap(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut v = Vec::new();
        snapshot(root, root, &mut v);
        v.sort();
        v
    }

    fn seed(tag: &str) -> (PathBuf, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base =
            std::env::temp_dir().join(format!("apmerge-{tag}-{}-{nanos}", std::process::id()));
        let src = base.join("src");
        let _dst = base.join("dst");
        std::fs::create_dir_all(src.join("pkg/sub")).expect("main.rs:586");
        std::fs::create_dir_all(src.join("pkg-1.0.dist-info")).expect("main.rs:587");
        std::fs::write(src.join("pkg/__init__.py"), b"__version__ = '1.0'\n").expect("main.rs:588");
        std::fs::write(src.join("pkg/sub/mod.py"), b"X = 1\n").expect("main.rs:589");
        std::fs::write(src.join("pkg-1.0.dist-info/METADATA"), b"Name: pkg\n")
            .expect("main.rs:590");
        std::fs::write(src.join("top.py"), b"print('top')\n").expect("main.rs:591");
        (base, src)
    }

    #[test]
    fn rename_merge_matches_copy_merge_on_fresh_dst() {
        let (base, src) = seed("fresh");
        // merge_dirs MOVES (rename), so the copy arm needs its own pristine
        // staging tree — same content, independent source.
        let (base2, src2) = seed("fresh-copy");
        let via_rename = base.join("dst-rename");
        let via_copy = base2.join("dst-copy");
        std::fs::create_dir_all(&via_rename).expect("main.rs:603");
        std::fs::create_dir_all(&via_copy).expect("main.rs:604");
        merge_dirs(&src, &via_rename).expect("rename merge");
        copy_tree(&src2, &via_copy).expect("copy merge");
        assert_eq!(snap(&via_rename), snap(&via_copy));
        assert_eq!(
            snap(&via_rename).len(),
            4,
            "merged set: pkg/__init__.py, pkg/sub/mod.py, dist-info/METADATA, top.py"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn collision_dirs_merge_and_file_overwrites() {
        // Re-add semantics: dst already has pkg/ with an extra installed
        // file, plus a stale top.py. Rename-merge must merge into pkg/
        // (keeping the extra file) and overwrite top.py — identical to the
        // old copy-merge's end state.
        let (base, src) = seed("collision");
        let (base2, src2) = seed("collision-copy");
        let via_rename = base.join("dst-rename");
        let via_copy = base2.join("dst-copy");
        for d in [&via_rename, &via_copy] {
            std::fs::create_dir_all(d.join("pkg")).expect("main.rs:627");
            std::fs::write(d.join("pkg/stay.py"), b"installed earlier\n").expect("main.rs:628");
            std::fs::write(d.join("top.py"), b"STALE\n").expect("main.rs:629");
        }
        merge_dirs(&src, &via_rename).expect("rename merge");
        copy_tree(&src2, &via_copy).expect("copy merge");
        assert_eq!(snap(&via_rename), snap(&via_copy));
        let s = snap(&via_rename);
        assert!(
            s.iter()
                .any(|(p, b)| p == "pkg/stay.py" && b == b"installed earlier\n"),
            "collision merge keeps pre-existing files: {s:?}"
        );
        assert!(
            s.iter()
                .any(|(p, b)| p == "top.py" && b == b"print('top')\n"),
            "new file must replace the stale one: {s:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
