//! Integration tests for `ap_parse::parse` over the fixture corpus
//! (`tests/fixtures/parse/`, shared with the separate-authorship test track).
//!
//! Gates (plan leaf 01.01):
//! - every valid fixture parses `Ok`;
//! - every invalid fixture fails with the contract diagnostic
//!   `error at line L, column C: <message>` at the expected position;
//! - the source map (`ModuleAst::line_col`) maps byte offsets to 1-based
//!   line/column on a multi-line fixture.

use std::path::Path;

use ruff_text_size::Ranged;

/// Fixture corpus lives at the workspace root, sibling to the crate.
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/parse");

fn fixture_names(prefix: &str) -> Vec<String> {
    let dir = Path::new(FIXTURE_DIR);
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read fixtures dir {}: {e}", FIXTURE_DIR))
        .map(|e| e.unwrap().file_name().to_str().unwrap().to_string())
        .filter(|n| n.starts_with(prefix) && n.ends_with(".py"))
        .collect();
    names.sort();
    names
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new(FIXTURE_DIR).join(name)).unwrap()
}

/// Expected (line, col) of the first ruff syntax error, established by probe
/// against ruff 0.0.4 pinned to PY314. If ruff's recovery strategy changes at
/// a version bump, these are the values to re-derive.
const INVALID_EXPECTATIONS: &[(&str, usize, usize, &str)] = &[
    (
        "invalid_unclosed_paren.py",
        2,
        1,
        "Expected `)`, found name",
    ),
    (
        "invalid_bad_indent.py",
        3,
        1,
        "unindent does not match any outer indentation level",
    ),
    (
        "invalid_positional_after_kwarg.py",
        1,
        8,
        "Positional argument cannot follow keyword argument",
    ),
    ("invalid_stray_bracket.py", 1, 5, "Expected an expression"),
    ("invalid_bad_fstring.py", 1, 10, "Expected an expression"),
];

#[test]
fn corpus_valid_programs_parse_ok() {
    let names = fixture_names("valid_");
    assert!(
        names.len() >= 50,
        "expected at least 50 valid fixtures, found {}: {:?}",
        names.len(),
        names
    );
    for name in &names {
        let src = read_fixture(name);
        match ap_parse::parse(&src) {
            Ok(ast) => {
                // Empty modules are legal, but every fixture has a body.
                assert!(
                    !ast.module().body.is_empty(),
                    "{name}: parsed to an empty module"
                );
            }
            Err(e) => panic!("{name}: expected Ok, got {e}"),
        }
    }
}

#[test]
fn corpus_invalid_programs_fail_at_expected_position() {
    assert_eq!(
        INVALID_EXPECTATIONS.len(),
        5,
        "expected exactly 5 invalid fixture expectations"
    );
    for (name, line, col, expected_msg) in INVALID_EXPECTATIONS {
        let src = read_fixture(name);
        let err = match ap_parse::parse(&src) {
            Err(e) => e,
            Ok(_) => panic!("{name}: expected parse failure, got Ok"),
        };
        assert_eq!(err.line, *line, "{name}: wrong line");
        assert_eq!(err.col, *col, "{name}: wrong column");
        assert_eq!(
            err.message, *expected_msg,
            "{name}: wrong diagnostic message"
        );
        // Contract diagnostic format (T-01): error at line L, column C: <message>
        assert_eq!(
            err.to_string(),
            format!("error at line {line}, column {col}: {expected_msg}"),
            "{name}: Display must match the contract diagnostic format"
        );
    }
}

#[test]
fn invalid_unclosed_paren_display_is_byte_exact() {
    let src = read_fixture("invalid_unclosed_paren.py");
    let err = ap_parse::parse(&src).unwrap_err();
    assert_eq!(
        err.to_string(),
        "error at line 2, column 1: Expected `)`, found name"
    );
}

#[test]
fn source_map_maps_offsets_on_multiline_fixture() {
    // Use a generated valid fixture with known line structure.
    let names = fixture_names("valid_");
    let name = names
        .iter()
        .find(|n| n == &&"valid_try_except_else_finally.py".to_string())
        .expect("valid_try_except_else_finally.py fixture missing");
    let src = read_fixture(name);
    let ast = ap_parse::parse(&src).expect("fixture must parse");

    // Byte 0 -> (1, 1).
    assert_eq!(ast.line_col(0), (1, 1));

    // Find the start of each line and check the round trip.
    let mut expected_line = 1usize;
    let mut pos = 0usize;
    for line_text in src.split_inclusive('\n') {
        assert_eq!(
            ast.line_col(pos),
            (expected_line, 1),
            "{name}: offset {pos} should be start of line {expected_line}"
        );
        pos += line_text.len();
        expected_line += 1;
    }
    // Offsets past EOF clamp to the last line.
    let (last_line, _) = ast.line_col(src.len());
    assert_eq!(last_line, expected_line);
}

#[test]
fn source_map_survives_multibyte_characters() {
    // Columns count Unicode scalar values, not bytes: the column of a
    // character after a multi-byte char is byte-offset-independent.
    // Layout: `s = "hélló"` + "\n" + `t = 1` + "\n"; é and ó are 2 bytes each.
    // Scalar positions: s=0 ' '=1 ==2 ' '=3 '"'=4 h=5 é=6 l=7 l=8 ó=9 '"'=10 \n=11
    // Byte offsets:      0        1  2     3     4    5  6     8   9   10     12
    let src = "s = \"h\u{e9}ll\u{f3}\"\nt = 1\n";
    let ast = ap_parse::parse(src).unwrap();
    assert_eq!(ast.line_col(0), (1, 1)); // 's'
    assert_eq!(ast.line_col(4), (1, 5)); // opening quote
    assert_eq!(ast.line_col(5), (1, 6)); // 'h'
    assert_eq!(ast.line_col(6), (1, 7)); // 'é'
    assert_eq!(ast.line_col(8), (1, 8)); // second 'l' (byte 8, scalar col 8)
    assert_eq!(ast.line_col(10), (1, 10)); // 'ó'
    assert_eq!(ast.line_col(12), (1, 11)); // closing quote
    assert_eq!(ast.line_col(14), (2, 1)); // 't' (newline is byte 13)
    assert_eq!(ast.line_col(18), (2, 5)); // '1'
}

#[test]
fn parse_error_is_std_error() {
    let err = ap_parse::parse("x = (").unwrap_err();
    let _: &dyn std::error::Error = &err;
}

#[test]
fn module_ast_exposes_source_and_ruff_ast() {
    let src = "value = 40 + 2\n";
    let ast = ap_parse::parse(src).unwrap();
    assert_eq!(ast.source(), src);
    assert_eq!(ast.module().body.len(), 1);
    // One statement: an assignment whose value is a binop.
    let stmt = &ast.module().body[0];
    assert_eq!(stmt.range().start().to_u32(), 0);
}
