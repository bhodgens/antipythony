//! Pure-wheel-only acceptance filter (04.01) + the native-component
//! diagnostic (compat-contract C-07/C-08/C-10 shapes, applied at ADD time
//! per Plan 04 §Design "fail early" / C-11 total refusal).
//!
//! Two detection layers over the staged install tree:
//! 1. FILE layer: any member ending `.so` / `.pyd` / `.dylib` / `.dll` (the
//!    same predicate T-14's fixture oracle uses, `_NATIVE_MEMBER`) or an
//!    `.a`/`.o` object file. This catches everything actually on disk after
//!    uv resolved the wheel set, including per-platform C wheels.
//! 2. TAG layer: any `*.dist-info/WHEEL` whose `Tag:` lines are not
//!    `*-none-any` (e.g. `cp314-cp314-macosx_...`). Pure wheels are
//!    `py2/py3-none-any`; `Root-Is-Purelib: false` corroborates.
//!
//! The diagnostic carries, verbatim per C-08/C-10: the `APX-CEXT` token,
//! the offending top-level package name (the "full module name" of the
//! binary component), the literal phrase `C extension`, the policy pointers
//! (`docs/compat-contract.md` clause C-07 and
//! `plans/antipythony/10-c-extensions-stretch/orchestrator.md`), a
//! `file:line:col` location shape (the offending file path with `:1:1` —
//! add-time refusals have no source line; the C-10 family requires the
//! triple shape), and the Plan-10 pointer the leaf text asks for.

use std::path::Path;

/// Extensions that mark a native (C-extension) component.
pub const NATIVE_EXTS: [&str; 6] = [".so", ".pyd", ".dylib", ".dll", ".a", ".o"];

/// The add-time refusal text. `module` is the full path of the first native
/// component found (its top-level package is derived for the message), per
/// C-08 "names the module, not 'a C module'".
pub fn cext_diagnostic(module: &str, package: &str) -> String {
    format!(
        "APX-CEXT: `{package}` ships a native component (`{module}` is a C extension \
         module); antipythony v1 refuses it. Pure-Python wheels only — native \
         components require Plan 10 (not yet implemented), see \
         docs/compat-contract.md C-07 and \
         plans/antipythony/10-c-extensions-stretch/orchestrator.md \
         ({module}:1:1)"
    )
}

/// Scan a staged install tree. `Ok(())` = pure; `Err(text)` = the add-time
/// diagnostic to print before exiting nonzero.
pub fn check_pure(dir: &Path) -> Result<(), String> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(err) => {
                return Err(format!(
                    "antipythony: cannot inspect {}: {err}",
                    d.display()
                ))
            }
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if name == "__pycache__" {
                    continue;
                }
                stack.push(p);
                continue;
            }
            if native_member(&name) {
                let package = top_level_package(&p, dir);
                return Err(cext_diagnostic(&p.to_string_lossy(), &package));
            }
            if name == "WHEEL"
                && p.parent()
                    .map(|x| x.to_string_lossy().ends_with(".dist-info"))
                    .unwrap_or(false)
            {
                if let Err(bad_tag) = check_wheel_tags(&p) {
                    let package = top_level_package(&p, dir);
                    return Err(cext_diagnostic(
                        &format!("{} ({bad_tag})", p.to_string_lossy()),
                        &package,
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The T-14 oracle predicate: `(?i)\.(so|dylib|pyd|dll|o)$` (we add `.a`).
pub fn native_member(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    NATIVE_EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// All WHEEL `Tag:` lines must be `*-none-any` for a pure wheel.
fn check_wheel_tags(wheel_meta: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(wheel_meta).map_err(|e| format!("unreadable: {e}"))?;
    let mut saw_tag = false;
    for line in text.lines() {
        if let Some(tag) = line.strip_prefix("Tag:") {
            saw_tag = true;
            let tag = tag.trim();
            let mut parts = tag.split('-');
            let (_impl, abi, platform) = (
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
            );
            if abi != "none" || platform != "any" {
                return Err(format!("non-pure wheel tag `{tag}`"));
            }
        }
    }
    if !saw_tag {
        return Err("WHEEL metadata has no Tag line".to_string());
    }
    Ok(())
}

/// Top-level package dir/file a native member belongs to (the C-08 "full
/// module name" root), relative to the install root.
fn top_level_package(p: &Path, root: &Path) -> String {
    match p.strip_prefix(root) {
        Ok(rel) => {
            let mut comps = rel.components();
            match comps.next() {
                Some(c) => c.as_os_str().to_string_lossy().into_owned(),
                None => p.to_string_lossy().into_owned(),
            }
        }
        Err(_) => p.to_string_lossy().into_owned(),
    }
}
