//! Leaf HARDEN2 probe: baseline panic reproduction for the remaining
//! planner fail_closed_gen arms. PRE-FIX evidence: at base (main@1cd873c,
//! pre-gate) every row here printed LOWER-PANIC with the planner reason
//! recorded in NEXT-HARDEN2.md. POST-fix: every row scan-rejects with
//! `Unsupported` (verified this session). The pinned regression rows live
//! in scan_vet_contract.rs::harden2_gen_planner_gates_reject_at_scan —
//! THIS file stays as the probe-first evidence and quick re-prober.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

const CASES: &[(&str, &str)] = &[
    // A: yield nested in a gen if-condition
    (
        "gen-if-cond-yield",
        "def g():\n    x = 1\n    if x + (yield 1):\n        pass\n",
    ),
    // B: yield nested in a gen while-condition
    (
        "gen-while-cond-yield",
        "def g():\n    x = 1\n    while x + (yield 1):\n        pass\n",
    ),
    // C: yield in a gen for-iter
    (
        "gen-for-iter-yield",
        "def g():\n    for i in (yield 1):\n        pass\n",
    ),
    // D: handler bind-yield
    (
        "handler-bind-yield",
        "def h():\n    try:\n        yield 1\n    except ValueError as e:\n        y = yield 2\n",
    ),
    // E: yield nested in a handler (deeper than a top-level ExprStmt)
    (
        "handler-nested-yield",
        "def h():\n    try:\n        yield 1\n    except ValueError:\n        if True:\n            yield 2\n",
    ),
    // F: split-yield lhs beyond a name (GE002 shape with a non-Name lhs)
    (
        "split-yield-lhs-attr",
        "def g():\n    o = A()\n    o.k = 1 + (yield 2)\n",
    ),
    // G: gen assign target beyond a name
    (
        "gen-assign-tuple-target",
        "def g():\n    a, b = 1, (yield 2)\n",
    ),
    // H: yield in gen return
    ("gen-return-yield", "def g():\n    return (yield 1)\n"),
    // I: break/continue outside a gen loop (yield in gen Try body is fine;
    //    a break with no enclosing gen loop)
    (
        "gen-break-outside-loop",
        "def g():\n    try:\n        yield 1\n        break\n    except ValueError:\n        pass\n",
    ),
    // J: gen stmt beyond the segment slice (AnnAssign inside a gen)
    ("gen-annassign", "def g():\n    x: int = (yield 1)\n"),
    // K: while-else inside a gen
    (
        "gen-while-else",
        "def g():\n    while False:\n        yield 1\n    else:\n        yield 2\n",
    ),
    // L: for-else inside a gen
    (
        "gen-for-else",
        "def g():\n    for i in range(1):\n        yield i\n    else:\n        yield 2\n",
    ),
    // M: attr assign target inside a gen (planner wants Name)
    (
        "gen-attr-assign-target",
        "def g():\n    o = 1\n    o.k = (yield 2)\n",
    ),
    // N: augassign with a yield value inside a gen
    (
        "gen-augassign-yield",
        "def g():\n    x = 1\n    x += (yield 1)\n",
    ),
    // O: nested def inside a generator body
    (
        "gen-nested-def",
        "def g():\n    def h():\n        return 1\n    yield h()\n",
    ),
    // P: match inside a generator body
    (
        "gen-match",
        "def g():\n    x = 1\n    match x:\n        case 1:\n            yield 2\n",
    ),
    // Q: yield nested beyond a top-level ExprStmt (call arg)
    (
        "gen-exprstmt-nested-yield",
        "def g():\n    print((yield 1))\n",
    ),
    // R: yield nested in an assign value (list literal)
    (
        "gen-assign-nested-yield",
        "def g():\n    x = [(yield 1)]\n",
    ),
    // S: split-yield lhs beyond a name (Call lhs)
    (
        "gen-split-yield-call-lhs",
        "def g():\n    x = f(1) + (yield 2)\n",
    ),
    // T: split-yield lhs beyond a name (Attribute lhs, resolvable)
    (
        "gen-split-yield-attr-lhs",
        "def g():\n    x = o.k + (yield 2)\n",
    ),
    // U: lambda whose body is a yield (registers <lambda> with is_generator
    // — the emit-side lambda lookup filters is_generator == false, so the
    // rposition .expect("scan registered this lambda") panics)
    (
        "gen-yield-lambda",
        "def g():\n    f = lambda: (yield 1)\n    yield f\n",
    ),
];

#[test]
fn probe_harden2_baselines() {
    for (name, src) in CASES {
        let (m, _) = match ap_parse::lower_source(src) {
            Ok(x) => x,
            Err(e) => {
                println!("{name}: PARSE-ERR {e:?}");
                continue;
            }
        };
        let n = name.to_string();
        let r = std::panic::catch_unwind(move || ap_codegen_wasm::lower_module_ex(&m, Some(&n)));
        match r {
            Ok(Ok(_)) => println!("{name}: LOWERS"),
            Ok(Err(ap_codegen_wasm::CodegenError::Unsupported(w))) => {
                println!("{name}: UNSUPPORTED {w}")
            }
            Ok(Err(e)) => println!("{name}: LOWER-ERR {e:?}"),
            Err(_) => println!("{name}: LOWER-PANIC"),
        }
    }
}
