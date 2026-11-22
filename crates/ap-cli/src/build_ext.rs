//! `antipythony build-ext` — compile a C extension source against the
//! antipythony C-API headers (Plan 10, leaf 10.02 spike -> 10.03 graduation).
//!
//! 10.03 surface (this file):
//! - module-name inference: the source's `PyInit_<name>` symbol wins over
//!   the file stem (they must agree; a mismatch fails loud — the runtime's
//!   dlopen probe keys on `lib<name>.so` + `PyInit_<name>`);
//! - output conventions: `-o` explicit path, else `lib<name>.so` next to
//!   the source (the ce_bridge dlopen probe's first candidate), or inside
//!   APX_EXT_DIR when set;
//! - linkage decision (recorded): extensions are UNRESOLVED shared objects —
//!   `cc -shared` with undefined Py* symbols left to resolve at dlopen time
//!   against the running `antipythony` binary, which exports them via
//!   `-Wl,-export_dynamic` (ap-cli/build.rs, the source-shim model). We do
//!   NOT link against the ap-aot-crt-hybrid staticlib: that CRT is the AoT
//!   program entry (rss-guard + catch_unwind + 256MB stack thread) and
//!   carries no Py* surface of its own; a second copy of the shim inside
//!   each extension would fork the ap-ce registries (two `apx_inits`
//!   tables) and break the whole loading model. The header + export-dynamic
//!   pair IS the linkage contract.
//! - `--emit-header`: install include/apx_python.h beside the output (the
//!   release-archive story mirrors scripts/package-release.sh, which ships
//!   include/apx_python.h since 10.03 — see that script + docs/build-ext.md).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::{Path, PathBuf};
use std::process::Command;

/// The module name for a source file: the `PyInit_<name>` symbol in the
/// source text when present (the loader's contract), else the file stem.
/// The two are cross-checked after extraction: a stem that disagrees with
/// an explicit PyInit_ symbol is a build error (the .so would be built
/// under a name the dlopen probe never asks for... unless -o names it
/// explicitly, which still leaves PyInit_<stem>-named imports broken).
fn module_name_of(src: &Path, text: &str) -> Result<String, String> {
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "antipythony: build-ext: source must be <name>.c-shaped".to_string())?
        .to_string();
    // Scan for PyInit_<ident>: the loader dlsym's exactly this. Take the
    // LAST definition-shaped occurrence (comment mentions lose to code).
    let mut inferred: Option<String> = None;
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text[i..].find("PyInit_") {
        let at = i + pos + "PyInit_".len();
        let mut end = at;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        if end > at {
            // Only a definition/declaration context counts: the char
            // BEFORE the "PyInit_" match must be whitespace, '*' or '('
            // (rejects `apx_fooPyInit_x`-style embedded names).
            let match_start = i + pos;
            let prev_ok = match_start
                .checked_sub(1)
                .map(|p| {
                    let c = bytes[p];
                    c.is_ascii_whitespace() || c == b'*' || c == b'('
                })
                .unwrap_or(true); // at buffer start counts as a boundary
            if prev_ok {
                inferred = Some(text[at..end].to_string());
            }
        }
        i = end;
    }
    match inferred {
        Some(name) if name == stem => Ok(name),
        Some(name) => Err(format!(
            "antipythony: build-ext: PyInit_{name} does not match the source \
             file name '{stem}' — rename the file to {name}.c (the loader \
             probes lib<{name}>.so for PyInit_{name})"
        )),
        None => {
            // No PyInit_ found: allow the stem anyway (the 10.02 spike built
            // this way via registration-only shape) but say so — the runtime
            // will fail at import with a clear PyInit-missing diagnostic.
            eprintln!(
                "antipythony: build-ext: warning: no PyInit_{stem} found in \
                 {stem}.c — the runtime will not import this module"
            );
            Ok(stem)
        }
    }
}

/// Locate the headers: relative to the binary's repo checkout (dev form),
/// or APX_INCLUDE env override.
fn include_dir() -> Result<PathBuf, String> {
    if let Some(d) = std::env::var_os("APX_INCLUDE") {
        return Ok(PathBuf::from(d));
    }
    // Dev layout: <repo>/include/apx_python.h; find via the exe path's
    // ancestors, then cwd.
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut p = exe.as_path();
        while let Some(parent) = p.parent() {
            roots.push(parent.to_path_buf());
            p = parent;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut p = cwd.as_path();
        while let Some(parent) = p.parent() {
            roots.push(parent.to_path_buf());
            p = parent;
        }
    }
    for r in roots {
        let cand = r.join("include").join("apx_python.h");
        if cand.is_file() {
            return Ok(r.join("include"));
        }
    }
    Err(
        "antipythony: build-ext: cannot locate include/apx_python.h \
         (set APX_INCLUDE=<dir>)"
            .to_string(),
    )
}

/// Run the compile. `-o out.so` names the output; `--emit-header` copies
/// apx_python.h beside it.
pub fn build_ext(rest: &[&str]) -> Result<(), String> {
    let mut src: Option<String> = None;
    let mut out: Option<String> = None;
    let mut emit_header = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "-o" => {
                i += 1;
                out = Some(
                    rest.get(i)
                        .ok_or("antipythony: build-ext: -o needs a path")?
                        .to_string(),
                );
            }
            "--emit-header" => emit_header = true,
            other if src.is_none() => src = Some(other.to_string()),
            other => {
                return Err(format!(
                    "antipythony: build-ext: unexpected argument '{other}'"
                ))
            }
        }
        i += 1;
    }
    let src = src.ok_or("usage: antipythony build-ext <src.c> [-o out.so] [--emit-header]")?;
    let src_path = Path::new(&src);
    if !src_path.is_file() {
        return Err(format!("antipythony: build-ext: no such file: {src}"));
    }
    let text = std::fs::read_to_string(src_path)
        .map_err(|e| format!("antipythony: build-ext: cannot read {src}: {e}"))?;
    let mod_name = module_name_of(src_path, &text)?;
    // Output convention (10.03): -o wins; else APX_EXT_DIR/lib<name>.so when
    // the env dir is set (the loader's late probe candidate); else
    // lib<name>.so beside the source (the loader's FIRST probe candidate —
    // build-where-you-import).
    let out_path: PathBuf = match out {
        Some(o) => PathBuf::from(o),
        None => match std::env::var_os("APX_EXT_DIR") {
            Some(d) => PathBuf::from(d).join(format!("lib{mod_name}.so")),
            None => src_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(format!("lib{mod_name}.so")),
        },
    };
    let include = include_dir()?;

    // Linkage contract (see module doc): undefined Py* symbols stay
    // undefined here and resolve at dlopen against the exporting runtime
    // binary. macOS wants -undefined dynamic_lookup; ELF wants nothing
    // (-shared leaves undefined symbols legal by default).
    let mut cmd = Command::new("cc");
    cmd.arg("-shared")
        .arg("-fPIC")
        .arg("-I")
        .arg(&include)
        .arg("-O2");
    if cfg!(target_os = "macos") {
        cmd.arg("-undefined").arg("dynamic_lookup");
    }
    cmd.arg(&src).arg("-o").arg(&out_path);
    let status = cmd
        .status()
        .map_err(|e| format!("antipythony: build-ext: cc: {e}"))?;
    if !status.success() {
        return Err("antipythony: build-ext: compilation failed".to_string());
    }
    if emit_header {
        let dst = out_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("apx_python.h");
        let hdr_src = include.join("apx_python.h");
        std::fs::copy(&hdr_src, &dst)
            .map_err(|e| format!("antipythony: build-ext: cannot copy header: {e}"))?;
        println!("antipythony: header installed -> {}", dst.display());
    }
    println!(
        "antipythony: built extension module '{mod_name}' -> {}",
        out_path.display()
    );
    Ok(())
}
