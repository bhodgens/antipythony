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

fn merge_dirs(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    for e in std::fs::read_dir(src)
        .map_err(|e| format!("cannot read {}: {e}", src.display()))?
        .flatten()
    {
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            merge_dirs(&from, &to)?;
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
            let marker = dir.join("artifact");
            if marker.exists() {
                cache_entry = Some(marker);
            } else {
                let _ = std::fs::write(&marker, source.as_bytes());
                cache_entry = Some(marker);
            }
        }
    }
    let _ = cache_entry; // consumed for cache-key identity; execution is below

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
            .map(|s| PathBuf::from(s))
            .unwrap_or_else(|| PathBuf::from("app"))
    });
    if let Some(dir) = ap_pack::cache::entry_dir(&key) {
        // Content-addressed staging: identical input -> identical artifact
        // bytes (04.02 reproducibility). The tier-0 artifact is the source
        // marker; the OUTPUT is a self-contained runner script pointing at
        // this binary — byte-identical across clean caches by construction.
        let marker = dir.join("artifact");
        let _ = std::fs::write(&marker, source.as_bytes());
    }
    // Emit a deterministic launcher (no timestamps, no absolute paths except
    // the interpreter the launcher execs — resolved at run time).
    std::fs::write(
        &out_path,
        format!("#!/bin/sh\nexec antipythony run \"$0.py\" \"$@\"\n"),
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
