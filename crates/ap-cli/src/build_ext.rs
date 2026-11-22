//! `antipythony build-ext` — the leaf 10.02 build verb: compile a C
//! extension source against the antipythony headers and link it as a
//! shared object the runtime can dlopen at import time.
//!
//! S0 spike form: cc -shared against the repo's include/ dir. The output
//! `.so` carries PyInit_<mod>; the runtime's import seam runs it through
//! the ap-ce registries (an embedded extension — the dylib loads with the
//! process binary exporting the Py* symbols, so dlopen resolves them).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::{Path, PathBuf};
use std::process::Command;

/// Module name from a source filename: spike.c -> spike.
fn module_name_of(src: &Path) -> Option<String> {
    src.file_stem()?.to_str().map(|s| s.to_string())
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

/// Run the compile. `-o out.so` names the output.
pub fn build_ext(rest: &[&str]) -> Result<(), String> {
    let mut src: Option<String> = None;
    let mut out: Option<String> = None;
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
            other if src.is_none() => src = Some(other.to_string()),
            other => {
                return Err(format!(
                    "antipythony: build-ext: unexpected argument '{other}'"
                ))
            }
        }
        i += 1;
    }
    let src = src.ok_or("usage: antipythony build-ext <src.c> [-o out.so]")?;
    let src_path = Path::new(&src);
    if !src_path.is_file() {
        return Err(format!("antipythony: build-ext: no such file: {src}"));
    }
    let mod_name =
        module_name_of(src_path).ok_or("antipythony: build-ext: source must be spike.c-shaped")?;
    let out_path = out.unwrap_or_else(|| format!("lib{mod_name}.so"));
    let include = include_dir()?;

    let status = Command::new("cc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("-I")
        .arg(&include)
        .arg("-O2")
        .arg("-undefined")
        .arg("dynamic_lookup")
        .arg(&src)
        .arg("-o")
        .arg(&out_path)
        .status()
        .map_err(|e| format!("antipythony: build-ext: cc: {e}"))?;
    if !status.success() {
        return Err("antipythony: build-ext: compilation failed".to_string());
    }
    println!("antipythony: built extension module '{mod_name}' -> {out_path}");
    Ok(())
}
