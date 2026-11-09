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
    // F1 (memperf audit 2026-11-06): read_dir order is filesystem-dependent.
    // With multiple native wheels staged (scrapy pulls lxml+brotli+
    // cryptography) the first-found early return named a different package
    // per machine/run. Deterministic contract fix: collect ALL hits, then
    // report the lexicographic-first by relative path. Diagnostic text for
    // a single offender is byte-identical to the old early-return form.
    let mut hits: Vec<(String, String)> = Vec::new(); // (rel-path, diagnostic)
    while let Some(d) = stack.pop() {
        let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(&d) {
            Ok(e) => e.flatten().collect(),
            Err(err) => {
                return Err(format!(
                    "antipythony: cannot inspect {}: {err}",
                    d.display()
                ))
            }
        };
        // Stable traversal so hit-collection order never matters.
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
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
                hits.push((
                    p.strip_prefix(dir)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .into_owned(),
                    cext_diagnostic(&p.to_string_lossy(), &package),
                ));
            } else if name == "WHEEL"
                && p.parent()
                    .map(|x| x.to_string_lossy().ends_with(".dist-info"))
                    .unwrap_or(false)
            {
                if let Err(bad_tag) = check_wheel_tags(&p) {
                    let package = top_level_package(&p, dir);
                    hits.push((
                        p.strip_prefix(dir)
                            .unwrap_or(&p)
                            .to_string_lossy()
                            .into_owned(),
                        cext_diagnostic(&format!("{} ({bad_tag})", p.to_string_lossy()), &package),
                    ));
                }
            }
        }
    }
    match hits.into_iter().min_by(|a, b| a.0.cmp(&b.0)) {
        Some((_, diag)) => Err(diag),
        None => Ok(()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// F1 (memperf audit 2026-11-06): with MULTIPLE native offenders staged,
    /// the refusal must name the lexicographic-first by relative path —
    /// never a filesystem-order-dependent pick.
    #[test]
    fn check_pure_reports_lexicographic_first_offender() {
        let root = std::env::temp_dir().join(format!("apx_purity_f1_{}", std::process::id()));
        let pkg_a = root.join("apkg_a/lib");
        let pkg_z = root.join("zpkg_b/lib");
        std::fs::create_dir_all(&pkg_a).expect("purity.rs:166");
        std::fs::create_dir_all(&pkg_z).expect("purity.rs:167");
        std::fs::write(pkg_z.join("zfirst.so"), b"").expect("purity.rs:168");
        std::fs::write(pkg_a.join("asecond.dylib"), b"").expect("purity.rs:169");

        let err = check_pure(&root).expect_err("purity.rs:171");
        assert!(
            err.contains("apkg_a"),
            "diagnostic must name the lexicographic-first offender (apkg_a), got: {err}"
        );
        std::fs::remove_dir_all(&root).expect("purity.rs:176");
    }

    #[test]
    fn check_pure_clean_tree_is_ok() {
        let root = std::env::temp_dir().join(format!("apx_purity_ok_{}", std::process::id()));
        std::fs::create_dir_all(root.join("pkg/__pycache__")).expect("purity.rs:182");
        std::fs::write(root.join("pkg/mod.py"), b"x = 1").expect("purity.rs:183");
        std::fs::write(root.join("pkg/__pycache__/mod.pyc"), b"").expect("purity.rs:184");
        assert!(check_pure(&root).is_ok());
        std::fs::remove_dir_all(&root).expect("purity.rs:186");
    }
}
