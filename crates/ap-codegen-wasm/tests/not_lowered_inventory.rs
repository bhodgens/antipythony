//! Leaf 03.07: NOT-LOWDERED inventory. Lowering-stage-only sweep over the
//! native corpus families (values, classes, exceptions, controlflow, gc,
//! threading, codecs_cjk, tier1-transparency, tier1-risk4, stdlib, fstrings).
//! Records per-case: lowers / UNSUPPORTED(<reason>) / parse error. Never
//! executes the lowered bytes (a panic aborts the whole test — panics are
//! caught by recording the family + last case via eprintln ordering).
//!
//! Diagnostic only (03.07 report input); NOT a gate. Run:
//!   cargo test -p ap-codegen-wasm --test not_lowered_inventory -- --nocapture
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|p| p.to_path_buf())
        .expect("crate at <root>/crates/ap-codegen-wasm")
}

#[test]
fn not_lowered_inventory() {
    let root = repo_root();
    let fams = [
        "values",
        "classes",
        "exceptions",
        "controlflow",
        "gc",
        "threading",
        "codecs_cjk",
        "tier1-transparency",
        "tier1-risk4",
        "stdlib",
    ];
    let mut lowers = 0usize;
    let mut unsupported = 0usize;
    let mut other = 0usize;
    let mut panic_family = String::new();
    for fam in fams {
        panic_family = fam.to_string();
        let dir = root.join("tests/corpus").join(fam);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<_> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
            .collect();
        paths.sort();
        for p in paths {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let Ok(src) = std::fs::read_to_string(&p) else {
                continue;
            };
            let (m, _) = match ap_parse::lower_source(&src) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("{fam}/{name}: PARSE-ERR {e:?}");
                    other += 1;
                    continue;
                }
            };
            // 03.07: the emitter has known panic paths (unreachable! on
            // scan-vet gaps, e.g. e015's Subscript(Attribute(...))). The
            // inventory must SURVIVE them: lower each case behind
            // catch_unwind and record LOWER-PANIC as its own (blocker-class)
            // status instead of aborting the sweep.
            let name_owned = name.clone();
            let fam_owned = fam.to_string();
            let result = std::panic::catch_unwind(move || {
                ap_codegen_wasm::lower_module_ex(&m, Some(&name_owned))
            });
            match result {
                Ok(Ok(_)) => {
                    lowers += 1;
                    eprintln!("{fam}/{name}: LOWERS");
                }
                Ok(Err(ap_codegen_wasm::CodegenError::Unsupported(w))) => {
                    unsupported += 1;
                    eprintln!("{fam}/{name}: UNSUPPORTED {w}");
                }
                Ok(Err(e)) => {
                    other += 1;
                    eprintln!("{fam}/{name}: LOWER-ERR {e:?}");
                }
                Err(_) => {
                    other += 1;
                    eprintln!("{fam_owned}/{name}: LOWER-PANIC (emitter unreachable — 03.07 blocker class)");
                }
            }
        }
    }
    eprintln!(
        "INVENTORY: lowers={lowers} unsupported={unsupported} other={other} (panic family, if any: {panic_family})"
    );
}
