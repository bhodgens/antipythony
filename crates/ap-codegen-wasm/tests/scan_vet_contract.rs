//! Leaf HARDEN (post-03 emitter hardening): THE SCAN-VET CONTRACT.
//!
//! For each of the 11 LOWER-PANIC cases found by leaf 03.07
//! (plans/antipythony/03-wasm-backend/03.07-not-lowered.json,
//! totals.LOWER-PANIC = 11): scan + lower must either SUCCEED or return
//! `CodegenError::Unsupported` — NEVER panic. A panic here means the scan
//! accepted a source shape the emitter cannot lower (a loud `unreachable!`
//! process abort at lowering time), violating the project's fail-closed
//! core invariant. Any future scan/emit drift fails in this test.
//!
//! Per-case dispositions (all scan-rejected; reasons name the owning
//! post-03 slice, see NEXT-HARDEN.md):
//! - subscript reads (c001/c009/c016/e008/t1_004/V043) → "subscript read
//!   (post-03 emitter surface)"
//! - MRO multi-base diamonds (c019/c020/c022) → "multi-base class
//!   (post-03 C3/MRO surface)"
//! - gen try/finally (G001) → "generator try orelse/finally (03.04e
//!   segment slice)"
//! - unresolvable fn-scope free name (V063 `eval` in a genexp) →
//!   "unresolvable free name in fn scope (post-03 emitter surface)"
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

/// The 11 LOWER-PANIC rows from 03.07-not-lowered.json (family, case).
const LOWER_PANIC_CASES: [(&str, &str); 11] = [
    ("values", "V043-dict-key-errors.py"),
    ("values", "V063-str-repr-quotes-escapes.py"),
    ("classes", "c001_class_basics.py"),
    ("classes", "c009_nondata_shadowed.py"),
    ("classes", "c016_slots_messages.py"),
    ("classes", "c019_mro_examples.py"),
    ("classes", "c020_mro_errors.py"),
    ("classes", "c022_super_diamond_chase.py"),
    ("exceptions", "e008_exceptiongroup_structure.py"),
    ("controlflow", "G001-pep342-generators.py"),
    ("tier1-transparency", "t1_004_osr_side_effects_once.py"),
];

#[test]
fn scan_vet_contract_never_panics() {
    let root = repo_root();
    let mut rejected = 0usize;
    let mut lowered = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (fam, name) in LOWER_PANIC_CASES {
        let p = root.join("tests/corpus").join(fam).join(name);
        let src = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("corpus case missing {p:?}: {e}"));
        let (m, _) = ap_parse::lower_source(&src)
            .unwrap_or_else(|e| panic!("{fam}/{name}: parse failed: {e:?}"));
        let n = name.to_string();
        // catch_unwind: a panic is the BUG this test exists to catch —
        // record it as a failure instead of aborting the sweep.
        let r = std::panic::catch_unwind(move || ap_codegen_wasm::lower_module_ex(&m, Some(&n)));
        match r {
            Ok(Ok(_)) => lowered += 1,
            Ok(Err(ap_codegen_wasm::CodegenError::Unsupported(w))) => {
                rejected += 1;
                eprintln!("{fam}/{name}: UNSUPPORTED {w}");
            }
            Ok(Err(e)) => failures.push(format!(
                "{fam}/{name}: non-Unsupported CodegenError (also a contract \
                 break — scan must classify): {e:?}"
            )),
            Err(_) => failures.push(format!(
                "{fam}/{name}: LOWER-PANIC — scan accepted a shape the \
                 emitter unreachable!s on (fail-closed contract violation)"
            )),
        }
    }
    eprintln!(
        "SCAN-VET CONTRACT: {}/11 handled (lowered {lowered}, \
         unsupported {rejected}, failures {})",
        lowered + rejected,
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "scan-vet contract violations:\n{}",
        failures.join("\n")
    );
}
