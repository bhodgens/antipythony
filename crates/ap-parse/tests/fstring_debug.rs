//! Debug-form (`{x=}`) f-string lowering tests — BUG-1 fix leaf.
//!
//! Pins the PEP 701 debug-form semantics against the corpus cases
//! `tests/corpus/fstrings/cases/f17..f21` (expected outputs pinned from
//! real CPython 3.14.7 in `tests/corpus/fstrings/expected/`):
//! - the literal debug text survives lowering on `FStrPart::Interp`;
//! - implied conversion `r` applies ONLY when debug text is present AND no
//!   explicit conversion AND no format spec (f19/f20 keep their conv/spec).
//!
//! Runtime rendering is NOT exercised here (no evaluator in this leaf); the
//! IR/dump capture is the contract. `ap-ir-dump --golden` round-trips the
//! debug spelling, and the corpus gate tests/fstrings_corpus pins CPython
//! output bytes for the later runtime.

use ap_ir::{Conversion, Expr, FStrPart};
use ap_parse::lower_source;

/// FString parts of the sole positional argument of the last expression
/// statement's call (the `print(f"...")` shape used throughout this file).
fn fstring_parts(src: &str) -> Vec<FStrPart> {
    let (module, _) = lower_source(src).expect("program must lower");
    let mut last = None;
    for item in &module.items {
        if let ap_ir::Item::Stmt(ap_ir::Stmt::ExprStmt(Expr::Call(_, args))) = item {
            for a in args {
                if let ap_ir::Arg::Pos(Expr::FString(parts)) = a {
                    last = Some(parts.clone());
                }
            }
        }
    }
    last.unwrap_or_else(|| panic!("no f-string call found in {src:?}"))
}

#[test]
fn f17_plain_debug_keeps_text_and_implies_r() {
    // `f"{x=}"` -> debug_text "x=", conv implied !r (CPython prints x=42).
    let parts = fstring_parts("x = 42\nprint(f\"{x=}\")\n");
    assert_eq!(parts.len(), 1);
    match &parts[0] {
        FStrPart::Interp {
            expr,
            conv,
            spec,
            debug_text,
        } => {
            assert!(matches!(expr, Expr::Name(n) if n == "x"));
            assert_eq!(debug_text.as_deref(), Some("x="));
            assert_eq!(*conv, Some(Conversion::R));
            assert!(spec.is_none());
        }
        other => panic!("expected Interp, got {other:?}"),
    }
}

#[test]
fn f18_spaced_debug_preserves_whitespace() {
    // `f"{x = }"` -> debug_text "x = " (inner spacing is real output).
    let parts = fstring_parts("x = 42\nprint(f\"{x = }\")\n");
    match &parts[0] {
        FStrPart::Interp {
            conv, debug_text, ..
        } => {
            assert_eq!(debug_text.as_deref(), Some("x = "));
            assert_eq!(*conv, Some(Conversion::R));
        }
        other => panic!("expected Interp, got {other:?}"),
    }
}

#[test]
fn f19_debug_with_spec_has_no_implied_r() {
    // `f"{x=:>10}"` -> spec kept verbatim, NO implied !r (CPython prints
    // `x=        42` — the text is not repr'd).
    let parts = fstring_parts("x = 42\nprint(f\"{x=:>10}\")\n");
    match &parts[0] {
        FStrPart::Interp {
            conv,
            spec,
            debug_text,
            ..
        } => {
            assert_eq!(debug_text.as_deref(), Some("x="));
            assert_eq!(*conv, None);
            assert!(spec.is_some());
        }
        other => panic!("expected Interp, got {other:?}"),
    }
}

#[test]
fn f20_debug_with_explicit_conv_keeps_conv() {
    // `f"{x=!r}"` -> explicit conversion kept (not doubled, not implied).
    let parts = fstring_parts("x = \"txt\"\nprint(f\"{x=!r}\")\n");
    match &parts[0] {
        FStrPart::Interp {
            conv, debug_text, ..
        } => {
            assert_eq!(debug_text.as_deref(), Some("x="));
            assert_eq!(*conv, Some(Conversion::R));
        }
        other => panic!("expected Interp, got {other:?}"),
    }
}

#[test]
fn f21_debug_expr_text_spans_full_source() {
    // `f"{d['k'] = }"` -> debug_text covers subscript source + spacing.
    let parts = fstring_parts("d = {\"k\": [1, 2]}\nprint(f\"{d['k'] = }\")\n");
    match &parts[0] {
        FStrPart::Interp {
            conv, debug_text, ..
        } => {
            assert_eq!(debug_text.as_deref(), Some("d['k'] = "));
            assert_eq!(*conv, Some(Conversion::R));
        }
        other => panic!("expected Interp, got {other:?}"),
    }
}

#[test]
fn non_debug_forms_have_no_debug_text() {
    // f11 shape: explicit conv without debug must NOT grow debug text or an
    // extra implied conversion; plain `{x}` stays conv-free.
    let conv_parts = fstring_parts("x = 1\nprint(f\"{x!r}\")\n");
    assert_eq!(conv_parts.len(), 1);
    match &conv_parts[0] {
        FStrPart::Interp {
            conv, debug_text, ..
        } => {
            assert!(debug_text.is_none(), "no debug text without `=`");
            assert_eq!(*conv, Some(Conversion::R));
        }
        other => panic!("expected Interp, got {other:?}"),
    }
    let plain_parts = fstring_parts("x = 1\nprint(f\"{x}\")\n");
    assert_eq!(plain_parts.len(), 1);
    match &plain_parts[0] {
        FStrPart::Interp {
            conv, debug_text, ..
        } => {
            assert!(debug_text.is_none(), "no debug text without `=`");
            assert_eq!(*conv, None);
        }
        other => panic!("expected Interp, got {other:?}"),
    }
}
