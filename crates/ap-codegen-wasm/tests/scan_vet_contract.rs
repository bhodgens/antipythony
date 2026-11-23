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
//! HARDEN2 (tier1-harden2, NEXT-HARDEN2.md): the REMAINING planner
//! fail_closed_gen arms — gen if/while/for condition yields, handler
//! bind/nested yields, gen-return yield, orphan break/continue, gen
//! assign/for-target/exprstmt/assign-value/split-lhs shapes, def/match
//! inside a gen body, and the yield-lambda — each gated at scan with
//! `(family, name) = ("hardening", <probe name>)`. These have NO corpus
//! case (a scan-rejected source cannot live in a dumped wasm family: the
//! corpus dump `.expect("lower")`s every case); their sources are INLINE
//! below, each proven a LOWER-PANIC at base by
//! probe_harden2_baselines.rs.
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

/// HARDEN2 rows: (probe name, inline source, expected rejection substring).
/// Every row here panicked at BASE (probe_harden2_baselines.rs, pre-fix):
/// planner `fail_closed_gen` / emit `unreachable!("scan vetted ...")`.
/// No corpus case carries these shapes (the CLI corpus dump requires every
/// family case to LOWER), so the sources live inline — the contract only
/// pins (scan accepts → lower panics) violations, not behavior.
const HARDEN2_GATES: [(&str, &str, &str); 21] = [
    // --- the leaf brief's named arms ---
    (
        "gen-if-cond-yield",
        "def g():\n    x = 1\n    if x + (yield 1):\n        pass\n",
        "yield in a generator if-condition",
    ),
    (
        "gen-while-cond-yield",
        "def g():\n    x = 1\n    while x + (yield 1):\n        pass\n",
        "yield in a generator while-condition/else",
    ),
    (
        "gen-for-iter-yield",
        "def g():\n    for i in (yield 1):\n        pass\n",
        "yield in a generator for-iter/else",
    ),
    (
        "handler-bind-yield",
        "def h():\n    try:\n        yield 1\n    except ValueError as e:\n        y = yield 2\n",
        "yield binding in an exception handler",
    ),
    (
        "handler-nested-yield",
        "def h():\n    try:\n        yield 1\n    except ValueError:\n        if True:\n            yield 2\n",
        "yield nested in an exception handler",
    ),
    // --- the OTHER planner arms the probe proved ungated ---
    (
        "gen-return-yield",
        "def g():\n    return (yield 1)\n",
        "yield in a generator return",
    ),
    (
        "gen-break-outside-loop",
        "def g():\n    try:\n        yield 1\n        break\n    except ValueError:\n        pass\n",
        "break/continue outside a generator loop",
    ),
    (
        "gen-while-else",
        "def g():\n    while False:\n        yield 1\n    else:\n        yield 2\n",
        "yield in a generator while-condition/else",
    ),
    (
        "gen-for-else",
        "def g():\n    for i in range(1):\n        yield i\n    else:\n        yield 2\n",
        "yield in a generator for-iter/else",
    ),
    (
        "gen-attr-assign-target",
        "def g():\n    o = 1\n    o.k = (yield 2)\n",
        "generator assign target beyond a name",
    ),
    (
        "gen-augassign-yield",
        "def g():\n    x = 1\n    x += (yield 1)\n",
        "yield in a generator augassign",
    ),
    (
        "gen-nested-def",
        "def g():\n    def h():\n        return 1\n    yield h()\n",
        "statement beyond the generator segment slice",
    ),
    (
        "gen-match",
        "def g():\n    x = 1\n    match x:\n        case 1:\n            yield 2\n",
        "statement beyond the generator segment slice",
    ),
    (
        "gen-exprstmt-nested-yield",
        "def g():\n    print((yield 1))\n",
        "yield nested beyond a top-level ExprStmt",
    ),
    (
        "gen-assign-nested-yield",
        "def g():\n    x = [(yield 1)]\n",
        "yield nested in an assign value",
    ),
    (
        "gen-split-yield-call-lhs",
        "def g():\n    x = f(1) + (yield 2)\n",
        "generator split-yield lhs beyond a name",
    ),
    (
        "gen-split-yield-attr-lhs",
        "def g():\n    x = o.k + (yield 2)\n",
        "generator split-yield lhs beyond a name",
    ),
    (
        "gen-assign-tuple-target",
        "def g():\n    a, b = 1, (yield 2)\n",
        "yield nested in an assign value",
    ),
    (
        "gen-yield-lambda",
        "def g():\n    f = lambda: (yield 1)\n    yield f\n",
        "yield nested in an assign value",
    ),
    // G001 closer()-family shapes WITHOUT orelse/finally still split at the
    // handler (the G001 corpus case covers orelse/finally; this covers the
    // plain-handler split surviving — LOWERS at base, must stay non-panic).
    (
        "gen-handler-toplevel-yield-ok",
        "def h():\n    try:\n        yield 1\n    except ValueError:\n        yield 2\n",
        "",
    ),
    (
        "gen-plain-while-yield-ok",
        "def g():\n    x = 2\n    while x > 0:\n        x = x - (yield x)\n    return 0\n",
        "",
    ),
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

#[test]
fn harden2_gen_planner_gates_reject_at_scan() {
    let mut lowered = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (name, src, expect_reason) in HARDEN2_GATES {
        let (m, _) =
            ap_parse::lower_source(src).unwrap_or_else(|e| panic!("{name}: parse failed: {e:?}"));
        let n = name.to_string();
        let r = std::panic::catch_unwind(move || ap_codegen_wasm::lower_module_ex(&m, Some(&n)));
        match r {
            Ok(Ok(_)) => {
                lowered += 1;
                if !expect_reason.is_empty() {
                    failures.push(format!(
                        "{name}: LOWERS but the probe proved a planner panic \
                         at base — the gate regressed"
                    ));
                }
            }
            Ok(Err(ap_codegen_wasm::CodegenError::Unsupported(w))) => {
                eprintln!("{name}: UNSUPPORTED {w}");
                if expect_reason.is_empty() {
                    failures.push(format!(
                        "{name}: UNSUPPORTED({w}) but this shape is IN the \
                         slice (GE002 echo/generator try handler) — the gate \
                         over-rejects"
                    ));
                } else if !w.contains(expect_reason) {
                    failures.push(format!(
                        "{name}: rejection reason {w:?} does not name the \
                         owning slice (expected substring {expect_reason:?})"
                    ));
                }
            }
            Ok(Err(e)) => failures.push(format!(
                "{name}: non-Unsupported CodegenError (scan must classify \
                 the construct): {e:?}"
            )),
            Err(_) => failures.push(format!(
                "{name}: LOWER-PANIC — scan accepted a shape the planner \
                 fail_closed_gen panics on (fail-closed contract violation)"
            )),
        }
    }
    let gated = HARDEN2_GATES.len() - lowered;
    eprintln!(
        "HARDEN2 GATES: {}/{} gated (lowered {lowered}, failures {})",
        gated,
        HARDEN2_GATES.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "HARDEN2 gen-gate contract violations:\n{}",
        failures.join("\n")
    );
}
