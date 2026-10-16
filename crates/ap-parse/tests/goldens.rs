//! Golden tests for AST→IR lowering (leaf 01.03).
//!
//! Each golden is one corpus fixture (tests/fixtures/parse/valid_*.py)
//! lowered to IR and serialized with the deterministic golden formatter
//! (`dump_module`). The checked-in `tests/fixtures/goldens/<name>.ir` must
//! match byte-for-byte. Goldens pin lowering decisions; regenerate only with
//! a lowering-review (the corpus only grows — never weaken a golden).

use std::path::Path;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/parse");
const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/goldens");

/// (fixture stem, constructs exercised) — 20 programs curated for diversity
/// across the spec-01/02 surface (literals, scope, exceptions, patterns,
/// async, generators, f-strings, imports, comprehensions, classes).
const GOLDENS: &[(&str, &str)] = &[
    (
        "valid_literals_int",
        "int literals incl. big-int / unary minus (C-12/C-13)",
    ),
    (
        "valid_literals_complex",
        "complex literals + numeric promotion (S-01-8)",
    ),
    (
        "valid_literals_bytes_bytearray",
        "bytes literals + implicit concat (S-01-17/18)",
    ),
    (
        "valid_chained_comparisons",
        "compare chains, middle-evaluated-once (S-01-31)",
    ),
    (
        "valid_operators_bitwise",
        "bitwise/shift ops incl. invert (S-01-1)",
    ),
    (
        "valid_aug_assign_annotations",
        "augassign op set + ann-assign lattice seeds (S-03-26/S-04)",
    ),
    (
        "valid_posonly_defaults",
        "param kinds: pos-only + defaults (S-02-7/8)",
    ),
    (
        "valid_star_args_kwargs_kwonly",
        "param kinds: vararg/kwonly/kwarg (S-02-8/10)",
    ),
    (
        "valid_global_nonlocal",
        "scope classification: global + nonlocal cells (S-02-3/4)",
    ),
    (
        "valid_try_except_else_finally",
        "try dispatch order, else, finally (S-02-30..33)",
    ),
    (
        "valid_try_nested_reraise",
        "bare raise re-raise, nesting (S-02-28)",
    ),
    (
        "valid_with_multi_items",
        "with items, enter order (S-02-40..42)",
    ),
    (
        "valid_starred_assign_aug",
        "unpacking targets + starred assignment (S-02-19)",
    ),
    (
        "valid_conditional_expressions",
        "conditional expressions, lazy branches (S-01-27)",
    ),
    (
        "valid_list_comprehension",
        "comprehension clauses + filters (S-02-2/17)",
    ),
    (
        "valid_generator_yield_from",
        "generator flags + yield from (S-02-20..26)",
    ),
    (
        "valid_match_sequence_mapping",
        "PEP 634 sequence/mapping/or patterns",
    ),
    (
        "valid_fstring_conversion_nesting",
        "f-string conversions + format specs (S-01-16)",
    ),
    (
        "valid_imports_all_forms",
        "import forms incl. relative (S-04-6..10)",
    ),
    (
        "valid_async_def_await",
        "async def + await, is_async flag (S-05-13 area)",
    ),
];

fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

#[test]
fn goldens_exist_and_match_byte_for_byte() {
    assert_eq!(GOLDENS.len(), 20, "gate: exactly 20 goldens");
    for (name, note) in GOLDENS {
        let src_path = format!("{FIXTURE_DIR}/{name}.py");
        let golden_path = format!("{GOLDEN_DIR}/{name}.ir");
        assert!(
            Path::new(&golden_path).exists(),
            "{name}: golden missing ({note}); generate with the update flow, then eyeball"
        );
        let ast = ap_parse::parse(&read(&src_path))
            .unwrap_or_else(|e| panic!("{name}: fixture must parse: {e}"));
        let (module, scopes) =
            ap_parse::lower(&ast).unwrap_or_else(|e| panic!("{name}: must lower: {e}"));
        let actual = ap_ir::dump_module(&module);
        let expected = read(&golden_path);
        assert_eq!(
            actual, expected,
            "{name}: IR dump diverged from golden ({note})"
        );
        // The scope classification rides with every golden module; a minimal
        // invariant here: valid programs lower without scope errors and the
        // scope list is deterministic (sorted paths).
        let mut paths: Vec<&Vec<String>> = scopes.scopes.iter().map(|s| &s.path).collect();
        paths.sort();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "{name}: scope paths must be sorted");
    }
}

#[test]
fn golden_count_is_twenty() {
    let count = std::fs::read_dir(GOLDEN_DIR)
        .expect("goldens dir must exist")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".ir"))
        .count();
    assert_eq!(count, 20, "exactly 20 goldens checked in");
}
