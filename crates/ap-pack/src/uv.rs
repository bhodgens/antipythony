//! The uv subprocess wrapper (04.01).
//!
//! uv is a standalone Apache-2.0 binary used ONLY as a subprocess (Plan 04
//! §Design: "subprocess use, never a library dependency ... keeps our binary
//! clean"). Absent uv = helpful error naming the install docs (G-4: exact
//! wording unspecced; the error names `uv` and the doc URL).
//!
//! Minimum version: G-5 (the leaf says "pin minimum version" without naming
//! one). Our pin: any uv whose `pip install --target` exists — that is every
//! 0.3+ release; we REQUIRE `uv --version` to parse as 0.x and print the
//! live version in diagnostics. The pin lives here so docs/packaging.md and
//! the code stay in sync (UV_MIN_MAJOR below).

use std::process::Command;

pub const UV_MIN_MAJOR: u64 = 0;

/// The uv install doc the missing-uv error points at.
pub const UV_INSTALL_DOCS: &str = "https://docs.astral.sh/uv/getting-started/installation/";

/// Locate uv on PATH; `Err` is the user-facing diagnostic (names uv and the
/// install docs — T-14 `test_add_without_uv_on_path_errors_helpfully`).
pub fn find_uv() -> Result<String, String> {
    // Respect a UV env override (documented), else PATH.
    if let Ok(v) = std::env::var("ANTIPTYHONY_UV") {
        if !v.is_empty() {
            let p = std::path::PathBuf::from(&v);
            if p.is_file() {
                return Ok(v);
            }
            return Err(format!(
                "antipythony: ANTIPTYHONY_UV={v} is not an executable file"
            ));
        }
    }
    // std has no `which`; walk PATH ourselves (mirrors the CLI-absence
    // policy in tests: uv must be reachable through the SAME PATH we see).
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let cand = std::path::Path::new(dir).join("uv");
        if cand.is_file() {
            return Ok(cand.to_string_lossy().into_owned());
        }
    }
    Err(format!(
        "antipythony: `uv` was not found on PATH.\n  \
         antipythony resolves and fetches dependencies by shelling out to uv \n  \
         (never vendored). Install it first:\n  \
         {UV_INSTALL_DOCS}\n  \
         e.g. `curl -LsSf https://astral.sh/uv/install.sh | sh`"
    ))
}

/// `uv --version` string (e.g. "uv 0.11.29 (901092ee1 ...)"); errors when
/// uv is absent or its version cannot be probed.
pub fn uv_version() -> Result<String, String> {
    let uv = find_uv()?;
    let out = Command::new(&uv)
        .arg("--version")
        .output()
        .map_err(|e| format!("antipythony: cannot run {uv}: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        return Err(format!("antipythony: `{uv} --version` failed: {text}"));
    }
    let first = text.lines().next().unwrap_or("").trim().to_string();
    if first.is_empty() {
        return Err(format!("antipythony: `{uv} --version` printed nothing"));
    }
    check_min_version(&first)?;
    Ok(first)
}

fn check_min_version(version_line: &str) -> Result<(), String> {
    // "uv 0.11.29 ..." -> 0
    let rest = version_line.strip_prefix("uv ").unwrap_or(version_line);
    let major: u64 = rest
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .ok_or_else(|| format!("antipythony: cannot parse uv version from `{version_line}`"))?;
    if major < UV_MIN_MAJOR {
        return Err(format!(
            "antipythony: uv {version_line} is too old (minimum {UV_MIN_MAJOR}.x); \
             see {UV_INSTALL_DOCS}"
        ));
    }
    Ok(())
}

/// Result of a staged install: the staging dir (to promote) plus the parsed
/// per-package versions uv reported.
pub struct Staged {
    pub dir: std::path::PathBuf,
    pub installed: Vec<(String, String)>,
}

/// `uv pip install --target <staging> <spec>` into a FRESH staging dir.
/// The caller inspects the staging tree with [`crate::purity`] BEFORE
/// promoting it into the live site-packages (add-time fail-loud, C-11:
/// a refused add leaves nothing behind).
pub fn install_staged(spec: &str, staging: &std::path::Path) -> Result<Staged, String> {
    install_staged_opts(spec, staging, false)
}

/// `--no-deps` variant: install exactly `spec` (space-separated requirement
/// specs) without resolving transitive dependencies. This is the escape hatch
/// for closures uv would poison with platform wheels (e.g. `scrapy` pulls
/// lxml/brotli/cryptography C builds, none of which import-time-block a
/// file:// spider): add the pure subset pin-by-pin.
pub fn install_staged_no_deps(specs: &str, staging: &std::path::Path) -> Result<Staged, String> {
    install_staged_opts(specs, staging, true)
}

fn install_staged_opts(
    spec: &str,
    staging: &std::path::Path,
    no_deps: bool,
) -> Result<Staged, String> {
    let uv = find_uv()?;
    if staging.exists() {
        std::fs::remove_dir_all(staging)
            .map_err(|e| format!("cannot clear staging {}: {e}", staging.display()))?;
    }
    std::fs::create_dir_all(staging)
        .map_err(|e| format!("cannot create staging {}: {e}", staging.display()))?;
    let mut cmd = Command::new(&uv);
    cmd.args([
        "pip",
        "install",
        "--target",
        &staging.to_string_lossy(),
        "--no-compile",
    ]); // no __pycache__ churn in our cache
    if no_deps {
        cmd.arg("--no-deps");
    }
    cmd.arg(spec);
    let out = cmd
        .output()
        .map_err(|e| format!("antipythony: cannot run {uv}: {e}"))?;
    if !out.status.success() {
        let mut msg = String::from_utf8_lossy(&out.stderr).into_owned();
        if msg.trim().is_empty() {
            msg = String::from_utf8_lossy(&out.stdout).into_owned();
        }
        return Err(format!(
            "antipythony: `uv pip install --target {} {spec}` failed (exit {}):\n{}",
            staging.display(),
            out.status.code().unwrap_or(-1),
            msg
        ));
    }
    Ok(Staged {
        dir: staging.to_path_buf(),
        installed: installed_versions(staging),
    })
}

/// Read the `*.dist-info/METADATA` Name/Version pairs from an installed
/// target tree (the lock's pins come from here, not from uv's stdout —
/// parsing human output is fragile).
pub fn installed_versions(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(info) = name.strip_suffix(".dist-info") else {
            continue;
        };
        let meta = e.path().join("METADATA");
        let Ok(text) = std::fs::read_to_string(&meta) else {
            continue;
        };
        let mut pkg = String::new();
        let mut ver = String::new();
        for line in text.lines() {
            if line.starts_with("Name:") {
                pkg = line["Name:".len()..].trim().to_string();
            } else if line.starts_with("Version:") {
                ver = line["Version:".len()..].trim().to_string();
            }
            if !pkg.is_empty() && !ver.is_empty() {
                break;
            }
        }
        if pkg.is_empty() {
            // dist-info dir name is {name}-{version}.dist-info — a usable
            // fallback when METADATA lacks the headers.
            if let Some((n, v)) = info.split_once('-') {
                pkg = n.to_string();
                ver = v.to_string();
            }
        }
        if !pkg.is_empty() {
            out.push((pkg, ver));
        }
    }
    out.sort();
    out
}
