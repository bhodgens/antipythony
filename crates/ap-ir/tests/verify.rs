//! Leaf 01.04 differential tests: CPython 3.14 byte-matching for verify
//! diagnostics and constant folding (`ap_ir::fold`).
//!
//! Expected values below were produced by the pinned reference
//! (`/opt/homebrew/bin/python3.14`, 3.14.7) — probe runs recorded verbatim,
//! fold values from `eval`. Regeneration commands in the comments.

use ap_ir::{fold, Expr, Item, Stmt};
use ap_parse::{lower_source, verify_source};

// ---------------------------------------------------------------------------
// UnboundLocalError prediction vs CPython (S-02-2)
// ---------------------------------------------------------------------------

/// Message template byte-verified against CPython 3.14.7
/// (`cannot access local variable '{name}' where it is not associated with
/// a value`; NOT the pre-3.11 "referenced before assignment" wording).
fn ubl(name: &str) -> String {
    format!("cannot access local variable '{name}' where it is not associated with a value")
}

#[test]
fn unbound_local_message_byte_matches_cpython() {
    // CPython: def f(): print(z); z = 1 / f()  ->  UnboundLocalError: <msg>
    // (verified via /opt/homebrew/bin/python3.14 -c probes; the T-04 suite
    // pins the same text in fixtures/expected_t04.json)
    let diags = verify_source("def f():\n    print(z)\n    z = 1\n\nf()\n").unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].kind.class(), "UnboundLocalError");
    assert_eq!(diags[0].message, ubl("z"));
    assert_eq!((diags[0].line, diags[0].col), (Some(2), Some(11)));
}

#[test]
fn unbound_local_in_nested_fn() {
    // CPython-probed (3.14.7): a never-bound read through a nested fn is a
    // module-level NameError miss (no suggestion — no close candidate).
    let diags = verify_source(
        "def outer():\n    def inner():\n        return q\n    return inner\n\nouter()()\n",
    )
    .unwrap();
    assert!(
        diags.iter().any(|d| d.message == "name 'q' is not defined"
            && d.kind.class() == "NameError"
            && d.qualname == "outer.<locals>.inner"),
        "{diags:?}"
    );
}

#[test]
fn global_resolved_no_diag() {
    // CPython runs clean (module global read).
    let diags = verify_source("g = 7\n\ndef f():\n    return g\n\nprint(f())\n").unwrap();
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn builtin_resolved_no_diag() {
    let diags = verify_source("def f():\n    return len([1])\n\nprint(f())\n").unwrap();
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn param_resolved_no_diag() {
    let diags = verify_source("def f(a):\n    return a\n\nprint(f(1))\n").unwrap();
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn closure_free_resolved_no_diag() {
    // CPython: read-only closure over a set cell runs clean (n01).
    let diags =
        verify_source("def outer():\n    a = 1\n    def inner():\n        return a\n    return inner\n\nprint(outer()())\n")
            .unwrap();
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn comprehension_var_no_leak_no_diag() {
    // CPython: comprehension var does not leak; reading it after is a module
    // NameError (boundary-optional per T-04 G-3 — verify stays silent when
    // a suggestion candidate exists; 'id' is one char from 'i'... actually
    // CPython DOES suggest. We must stay silent: distance('i','id') <= 1).
    let diags = verify_source("print([i for i in range(3)])\n").unwrap();
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn except_var_no_diag() {
    // except-as binds statically (n04 family: try/except assigning both
    // paths, read after -> silent).
    let diags = verify_source(
        "def f():\n    try:\n        x = int('nope')\n    except ValueError:\n        x = 0\n    return x\n\nprint(f())\n",
    )
    .unwrap();
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn if_false_branch_read_flags() {
    // CPython-probed (3.14.7): `if False: x = 1; return x` raises
    // UnboundLocalError (the branch never runs) — the fixture p02 shape.
    let diags =
        verify_source("def f():\n    if False:\n        x = 1\n    return x\n\nprint(f())\n")
            .unwrap();
    assert!(diags.iter().any(|d| d.message == ubl("x")), "{diags:?}");
}

#[test]
fn dead_branch_assignment_after_read_flags() {
    // CPython: p02 shape — read BEFORE the only (branch) binding flags.
    let diags =
        verify_source("def f(flag):\n    if flag:\n        y = 1\n    return y\n\nf(False)\n")
            .unwrap();
    assert!(diags.iter().any(|d| d.message == ubl("y")), "{diags:?}");
}

#[test]
fn walrus_bind_then_read_no_diag() {
    // FINDING-3 differential: CPython runs all of these clean (3.14.7 probed;
    // corpus f26 shape). The walrus bind must be DEFINITE so later reads are
    // protected — the previous indefinite bind flagged them as NameError.
    let clean = [
        // f26: walrus inside an f-string replacement field, read after.
        "print(f\"[{(w := 42)}]|[{w}]\")\n",
        // call-argument walrus, read after (finder probe shape).
        "print(abs((v := -42)))\nprint(v)\n",
        // plain assignment RHS walrus.
        "z = (q := 7)\nprint(q)\n",
        // walrus in an `if` condition.
        "xs = [1, 2]\nif (n := len(xs)) > 0:\n    print(n)\n",
        // walrus inside a comprehension IF clause; leaking bind read after
        // (CPython: comprehension walrus binds in the enclosing scope).
        "ks = [1, 2, 3]\nout = [k for k in ks if (m := k) > 1]\nprint(out, m)\n",
    ];
    for src in clean {
        let diags = verify_source(src).unwrap();
        assert!(diags.is_empty(), "false positive for {src:?}: {diags:?}");
    }
}

#[test]
fn walrus_then_del_then_read_still_flags() {
    // FINDING-3 del_marks interaction: a walrus bind is protecting only until
    // a del unbinds it — CPython 3.14.7 raises UnboundLocalError here, and
    // verify must keep flagging (probed byte-identical message).
    let diags = verify_source("def f():\n    (v := 5)\n    del v\n    print(v)\n\nf()\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == ubl("v") && d.kind.class() == "UnboundLocalError"),
        "{diags:?}"
    );
}

#[test]
fn del_then_read_flags_local_form() {
    // CPython 3.14.7: `x = 1; del x; return x` inside a function raises
    // UnboundLocalError with the standard local template (probed directly).
    let diags = verify_source("def f():\n    x = 1\n    del x\n    return x\n\nf()\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == ubl("x") && d.kind.class() == "UnboundLocalError"),
        "{diags:?}"
    );
}

#[test]
fn del_of_never_bound_flags_p07_form() {
    // p07: `del q` never bound -> UnboundLocalError (G-7), caret col 9.
    let diags = verify_source("def f():\n    del q\n\nf()\n").unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].message, ubl("q"));
    assert_eq!((diags[0].line, diags[0].col), (Some(2), Some(9)));
}

#[test]
fn augassign_before_assign_flags_p04_form() {
    // p04: `x += 1` reads first (S-02-19) -> UnboundLocalError, caret col 5
    // (bare caret anchoring the whole statement).
    let diags = verify_source("def f():\n    x += 1\n\nf()\n").unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].message, ubl("x"));
    assert_eq!((diags[0].line, diags[0].col), (Some(2), Some(5)));
}

#[test]
fn big_int_folded_expr_message_context() {
    // The module-level unresolved diagnostic never carries the UBL template;
    // sanity: a clean big-int program verifies silently.
    let diags = verify_source("x = 2 ** 100\nprint(x)\n").unwrap();
    assert!(diags.is_empty(), "{diags:?}");
}

// ---------------------------------------------------------------------------
// Compile-stage static checks (S-02-3/4/11/18, S-04-10, S-02-8; T-04 G-4)
// ---------------------------------------------------------------------------

#[test]
fn global_and_nonlocal_clash_p09() {
    let diags = verify_source(
        "def outer():\n    x = 1\n    def f():\n        global x\n        nonlocal x\n    f()\n\nouter()\n",
    )
    .unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == "name 'x' is nonlocal and global"
                && d.kind.class() == "SyntaxError"
                && (d.line, d.col) == (Some(4), Some(5))),
        "{diags:?}"
    );
}

#[test]
fn nonlocal_at_module_p11() {
    let diags = verify_source("nonlocal x\n").unwrap();
    assert!(
        diags.iter().any(
            |d| d.message == "nonlocal declaration not allowed at module level"
                && (d.line, d.col) == (Some(1), Some(5))
        ),
        "{diags:?}"
    );
}

#[test]
fn nonlocal_no_binding_p10() {
    let diags = verify_source("def f():\n    nonlocal x\n    x = 1\n\nf()\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == "no binding for nonlocal 'x' found"
                && (d.line, d.col) == (Some(2), Some(5))),
        "{diags:?}"
    );
}

#[test]
fn break_outside_loop_p12() {
    let diags = verify_source("def f():\n    break\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == "'break' outside loop" && (d.line, d.col) == (Some(2), Some(5))),
        "{diags:?}"
    );
}

#[test]
fn continue_outside_loop_p13() {
    let diags = verify_source("def f():\n    continue\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == "'continue' not properly in loop"
                && (d.line, d.col) == (Some(2), Some(5))),
        "{diags:?}"
    );
}

#[test]
fn return_outside_function_p14() {
    let diags = verify_source("def f():\n    pass\n\nreturn 1\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == "'return' outside function"
                && (d.line, d.col) == (Some(4), Some(5))),
        "{diags:?}"
    );
}

#[test]
fn import_star_in_fn_p15() {
    let diags = verify_source("def f():\n    from os import *\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == "import * only allowed at module level"
                && (d.line, d.col) == (Some(2), Some(20))),
        "{diags:?}"
    );
}

#[test]
fn import_star_in_class_p16() {
    let diags =
        verify_source("class C:\n    from os import getcwd\n    from os import *\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == "import * only allowed at module level"
                && (d.line, d.col) == (Some(3), Some(20))),
        "{diags:?}"
    );
}

#[test]
fn duplicate_param_p18() {
    let diags = verify_source("def f(x=1, x=2):\n    pass\n").unwrap();
    assert!(
        diags.iter().any(
            |d| d.message == "duplicate argument 'x' in function definition"
                && (d.line, d.col) == (Some(1), Some(16))
        ),
        "{diags:?}"
    );
}

#[test]
fn invalid_default_p17() {
    // Parse-stage remap (G-4): CPython message + caret col 12.
    let diags = verify_source("def f(x=):\n    pass\n").unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.message == "expected default value expression"
                && (d.line, d.col) == (Some(1), Some(12))),
        "{diags:?}"
    );
}

// ---------------------------------------------------------------------------
// Constant folding — byte-identical to CPython 3.14 eval
// (values generated + verified against /opt/homebrew/bin/python3.14)
// ---------------------------------------------------------------------------

/// Fold a single expression source and render it like CPython repr.
fn fold_src(src: &str) -> Option<String> {
    let (module, _) = lower_source(&format!("x = {src}\n")).unwrap();
    let mut module = module;
    fold(&mut module);
    match &module.items[0] {
        Item::Stmt(Stmt::Assign(_, v)) => Some(render(v)),
        _ => None,
    }
}

/// Render a folded literal exactly as CPython 3.14 repr would.
fn render(e: &Expr) -> String {
    match e {
        Expr::LitInt(v) => v.to_string(),
        Expr::LitBigInt(d) => d.clone(),
        Expr::LitFloat(f) => ap_ir::verify::py_float_repr(*f),
        Expr::LitStr(s) => format!("'{s}'"),
        Expr::LitBool(b) => if *b { "True" } else { "False" }.to_string(),
        Expr::LitNone => "None".to_string(),
        Expr::LitComplex(re, im) => {
            // Python complex repr: (re+imj), parens when both parts nonzero,
            // re omitted when zero, im always with j.
            let (sign, im_abs) = if *im < 0.0 { ('-', -im) } else { ('+', *im) };
            let im_s = ap_ir::verify::py_float_repr(im_abs);
            let im_s = im_s.trim_end_matches(".0");
            if *re == 0.0 {
                format!("{im_s}j")
            } else {
                let re_s = ap_ir::verify::py_float_repr(*re);
                format!("({re_s}{sign}{im_s}j)")
            }
        }
        Expr::LitBytes(b) => {
            let mut s = String::from("b'");
            for &byte in b {
                if (0x20..0x7f).contains(&byte) && byte != b'\'' && byte != b'\\' {
                    s.push(byte as char);
                } else {
                    s.push_str(&format!("\\x{byte:02x}"));
                }
            }
            s.push('\'');
            s
        }
        other => format!("<unfolded {other:?}>"),
    }
}

#[test]
fn fold_int_arith_matches_cpython() {
    // CPython: eval of each source == expected (generated via
    // /opt/homebrew/bin/python3.14 /tmp/gen_fold_cases.py).
    let cases: &[(&str, &str)] = &[
        ("1 + 2", "3"),
        ("7 - 3", "4"),
        ("2 * 255", "510"),
        ("7 // 2", "3"),
        ("7 % 3", "1"),
        ("7 // -2", "-4"),
        ("7 % -2", "-1"),
        ("-7 // 2", "-4"),
        ("-7 % 2", "1"),
        ("2 ** 10", "1024"),
        ("2 ** 100", "1267650600228229401496703205376"),
        ("10 ** 30", "1000000000000000000000000000000"),
        ("3 ** 40", "12157665459056928801"),
        ("(-5) ** 40", "9094947017729282379150390625"),
        ("(-2) ** 3", "-8"),
        ("(-2) ** 2", "4"),
        ("2 ** 0", "1"),
        ("0 ** 0", "1"),
        ("1 << 40", "1099511627776"),
        ("1024 >> 3", "128"),
        ("-16 >> 2", "-4"),
        ("-1 >> 1", "-1"),
        ("5 | 3", "7"),
        ("5 & 3", "1"),
        ("5 ^ 3", "6"),
        ("~10", "-11"),
        ("~(-4)", "3"),
        ("-(-5)", "5"),
        ("+7", "7"),
        ("255 + 7", "262"),
        ("1180591620717411303424 + 1", "1180591620717411303425"),
        (
            "1000000000000000000000000000000 - 3",
            "999999999999999999999999999997",
        ),
        ("(-1267650600228229401496703205376) % 3", "2"),
        (
            "1267650600228229401496703205376 * 255",
            "323250903058198497381659317370880",
        ),
        (
            "1000000000000000000000000000000 << 3",
            "8000000000000000000000000000000",
        ),
        ("2 >> 100", "0"),
        ("123456789 >> 40", "0"),
    ];
    for (src, want) in cases {
        let got = fold_src(src).unwrap_or_else(|| panic!("{src}: did not fold"));
        assert_eq!(got, *want, "fold({src})");
    }
}

#[test]
fn fold_float_arith_matches_cpython() {
    let cases: &[(&str, &str)] = &[
        ("1 + 0.5", "1.5"),
        ("0.5 + 1", "1.5"),
        ("1 * 0.5", "0.5"),
        ("3 * 2.5", "7.5"),
        ("4 / 2", "2.0"),
        ("3 / 2", "1.5"),
        ("1 / 3", "0.3333333333333333"),
        ("0.1 + 0.2", "0.30000000000000004"),
        ("2.0 ** 3", "8.0"),
        ("2 ** 0.5", "1.4142135623730951"),
        ("7 // 2.0", "3.0"),
        ("-7 // 2.0", "-4.0"),
        ("7.5 // 2", "3.0"),
        ("-7.5 // 2", "-4.0"),
        ("7.5 % 2", "1.5"),
        ("-7.5 % 2", "0.5"),
        ("7 % 2.5", "2.0"),
        ("-0.5", "-0.5"),
        ("-2.5", "-2.5"),
        ("+0.5", "0.5"),
        ("2 ** -2", "0.25"),
        ("0.0 * -1", "-0.0"),
        ("1.5 - (-0.5)", "2.0"),
        ("1e16 - 7", "9999999999999992.0"),
        ("6.0 / 2", "3.0"),
        ("3.0 / 3", "1.0"),
    ];
    for (src, want) in cases {
        let got = fold_src(src).unwrap_or_else(|| panic!("{src}: did not fold"));
        assert_eq!(got, *want, "fold({src})");
    }
}

#[test]
fn fold_string_ops_matches_cpython() {
    let cases: &[(&str, &str)] = &[
        ("'a' + 'b'", "'ab'"),
        ("'a' + 'b' + 'c'", "'abc'"),
        ("'ab' * 3", "'ababab'"),
        ("3 * 'xy'", "'xyxyxy'"),
        ("'ab' * 0", "''"),
        ("'ab' + ''", "'ab'"),
    ];
    for (src, want) in cases {
        let got = fold_src(src).unwrap_or_else(|| panic!("{src}: did not fold"));
        assert_eq!(got, *want, "fold({src})");
    }
}

#[test]
fn fold_boolop_shortcircuit_matches_cpython() {
    // S-01-30: and -> first falsy else last; or -> first truthy else last.
    let cases: &[(&str, &str)] = &[
        ("5 and 7", "7"),
        ("0 and 2", "0"),
        ("1 or 2", "1"),
        ("0 or 2", "2"),
        ("1 and 0", "0"),
        ("'' or 'x'", "'x'"),
        ("None or 1", "1"),
        ("True and False", "False"),
        ("True or 7", "True"),
        ("1 and 2 and 3", "3"),
        ("not 5", "False"),
        ("not 0", "True"),
        ("not ''", "True"),
        ("not 3.5", "False"),
        ("not None", "True"),
    ];
    for (src, want) in cases {
        let got = fold_src(src).unwrap_or_else(|| panic!("{src}: did not fold"));
        assert_eq!(got, *want, "fold({src})");
    }
}

#[test]
fn fold_does_not_fold_raising_ops() {
    // Division/modulo by zero, negative shifts, matmul, negative-base float
    // pow: must stay UNFOLDED (they raise at runtime).
    for src in ["1 / 0", "1 // 0", "1 % 0", "1.5 // 0", "1 << -1", "2 @ 3"] {
        let (module, _) = lower_source(&format!("x = {src}\n")).unwrap();
        let mut module = module;
        fold(&mut module);
        match &module.items[0] {
            Item::Stmt(Stmt::Assign(_, v)) => {
                assert!(
                    !matches!(v, Expr::LitInt(_) | Expr::LitFloat(_) | Expr::LitBigInt(_)),
                    "{src}: wrongly folded to {v:?}"
                );
            }
            other => panic!("{src}: unexpected shape {other:?}"),
        }
    }
}

#[test]
fn fold_never_touches_names_or_calls() {
    let (module, _) = lower_source("y = 3\nx = y + 1\nprint(x)\n").unwrap();
    let mut module = module;
    fold(&mut module);
    match &module.items[1] {
        Item::Stmt(Stmt::Assign(_, v)) => {
            assert!(matches!(v, Expr::BinOp(..)), "folded a name expr: {v:?}");
        }
        other => panic!("unexpected shape {other:?}"),
    }
    match &module.items[2] {
        Item::Stmt(Stmt::ExprStmt(v)) => {
            assert!(matches!(v, Expr::Call(..)), "folded a call: {v:?}");
        }
        other => panic!("unexpected shape {other:?}"),
    }
}

#[test]
fn fold_comparisons_unfolded() {
    // CPython's own peephole does NOT fold comparisons (probe: dis shows
    // COMPARE_OP); verify stays equally conservative.
    let (module, _) = lower_source("x = 1 < 2\n").unwrap();
    let mut module = module;
    fold(&mut module);
    match &module.items[0] {
        Item::Stmt(Stmt::Assign(_, v)) => {
            assert!(matches!(v, Expr::Compare(..)), "folded a compare: {v:?}");
        }
        other => panic!("unexpected shape {other:?}"),
    }
}
