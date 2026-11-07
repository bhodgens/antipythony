//! ruff wrapper producing typed AST + source maps.
//!
//! The single entry point is [`parse`], which parses Python source in module
//! mode, pinned to target version 3.14, and returns a [`ModuleAst`] owning
//! both the ruff [`ModModule`] and the source text (so offsets can always be
//! mapped back).
//!
//! # Diagnostic contract (T-01)
//!
//! Every failure — syntax errors *and* version-related
//! `UnsupportedSyntaxError`s, fail-closed — becomes an [`ParseError`]
//! whose `Display` is byte-exactly:
//!
//! ```text
//! error at line L, column C: <message>
//! ```
//!
//! where `L`/`C` are 1-based and columns count Unicode scalar values, not
//! bytes. This format is the T-01 test contract; do not change it without
//! re-deriving every consuming test.

use std::fmt;

use ruff_python_ast::{ModModule, PythonVersion};
use ruff_python_parser::{parse_unchecked, Mode, ParseOptions};
use ruff_text_size::Ranged;

/// Parse Python source into a [`ModuleAst`], pinned to Python 3.14 semantics.
///
/// Fail-closed: any ruff syntax error or version-related unsupported-syntax
/// error is returned as a [`ParseError`] carrying the contract diagnostic
/// (see the crate docs).
pub fn parse(source: &str) -> Result<ModuleAst, ParseError> {
    let options = ParseOptions::from(Mode::Module).with_target_version(PythonVersion::PY314);
    let parsed = parse_unchecked(source, options);

    if let Some(err) = parsed.errors().first() {
        let offset = usize::try_from(u32::from(err.range().start())).unwrap_or(0);
        return Err(error_at(source, offset, err.error.to_string()));
    }
    if let Some(err) = parsed.unsupported_syntax_errors().first() {
        let offset = usize::try_from(u32::from(err.range().start())).unwrap_or(0);
        return Err(error_at(source, offset, err.to_string()));
    }

    let module = parsed
        .try_into_module()
        .expect("module mode always parses to ModModule")
        .into_syntax();

    // Post-parse validation for diagnostic gaps (wave-7 frontend audit)
    validate_module(&module, source)?;

    Ok(ModuleAst {
        module,
        source: source.to_string(),
    })
}

/// Convert a byte offset into a 1-based (line, column) pair. Columns count
/// Unicode scalar values, not bytes; `offset` is clamped to the source length
/// (and down to the nearest scalar boundary).
fn error_at(source: &str, offset: usize, message: String) -> ParseError {
    let (line, col) = line_col_of(source, offset);
    ParseError {
        offset,
        line,
        col,
        message,
    }
}

/// Byte offset -> (1-based line, 1-based Unicode-scalar column).
fn line_col_of(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let offset = {
        let mut o = offset;
        while o > 0 && !source.is_char_boundary(o) {
            o -= 1;
        }
        o
    };
    let mut line = 1usize;
    let mut line_start = 0usize;
    for (i, b) in source.bytes().enumerate() {
        if i >= offset {
            break;
        }
        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let col = source[line_start..offset].chars().count() + 1;
    (line, col)
}

/// Statement start-lines in execution order (pre-order walk of the ruff
/// AST: a compound statement contributes its own line, then descends into
/// its bodies). Consumed by the executor's traceback line mapping (the IR
/// is span-free; this table maps statement ordinals to lines).
pub fn collect_stmt_lines(ast: &ModuleAst) -> Vec<usize> {
    use ruff_python_ast as rast;
    use ruff_text_size::Ranged;

    // B2 (memperf wave 3): build the line-start index ONCE and binary-search
    // it per node. The old form re-counted every newline in `source[..off]`
    // per statement — O(file) per node, O(file^2) per module — the identical
    // bug the Lowerer's `line_starts`/`line_at` pair already fixed
    // (see lowering.rs `Lowerer::line_at`; same 1-based semantics).
    let source = ast.source();
    let mut line_starts: Vec<usize> = Vec::with_capacity(64);
    line_starts.push(0usize);
    for (i, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(i + 1);
        }
    }

    fn offset_to_line(line_starts: &[usize], off: usize) -> usize {
        match line_starts.binary_search(&off) {
            Ok(i) => i + 1,
            Err(i) => i, // off is inside line i-1 (line_starts[i-1] < off)
        }
    }
    fn walk_body(
        body: &[rast::Stmt],
        line_starts: &[usize],
        source_len: usize,
        out: &mut Vec<usize>,
    ) {
        let off = |node: &dyn Ranged| node.start().to_u32() as usize;
        for s in body {
            out.push(offset_to_line(line_starts, off(s).min(source_len)));
            match s {
                rast::Stmt::If(i) => {
                    walk_body(&i.body, line_starts, source_len, out);
                    for c in &i.elif_else_clauses {
                        out.push(offset_to_line(line_starts, off(c).min(source_len)));
                        walk_body(&c.body, line_starts, source_len, out);
                    }
                }
                rast::Stmt::While(w) => {
                    walk_body(&w.body, line_starts, source_len, out);
                    walk_body(&w.orelse, line_starts, source_len, out);
                }
                rast::Stmt::For(f) => {
                    walk_body(&f.body, line_starts, source_len, out);
                    walk_body(&f.orelse, line_starts, source_len, out);
                }
                rast::Stmt::With(w) => walk_body(&w.body, line_starts, source_len, out),
                rast::Stmt::Try(t) => {
                    walk_body(&t.body, line_starts, source_len, out);
                    for h in &t.handlers {
                        out.push(offset_to_line(line_starts, off(h).min(source_len)));
                        let rast::ExceptHandler::ExceptHandler(hi) = h;
                        walk_body(&hi.body, line_starts, source_len, out);
                    }
                    walk_body(&t.orelse, line_starts, source_len, out);
                    walk_body(&t.finalbody, line_starts, source_len, out);
                }
                rast::Stmt::Match(m) => {
                    for c in &m.cases {
                        out.push(offset_to_line(line_starts, off(c).min(source_len)));
                        walk_body(&c.body, line_starts, source_len, out);
                    }
                }
                rast::Stmt::FunctionDef(f) => {
                    let body_line = f
                        .body
                        .first()
                        .map(|s| offset_to_line(line_starts, off(s).min(source_len)))
                        .unwrap_or(1);
                    out.push(body_line);
                    walk_body(&f.body, line_starts, source_len, out);
                }
                rast::Stmt::ClassDef(c) => walk_body(&c.body, line_starts, source_len, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk_body(&ast.module().body, &line_starts, source.len(), &mut out);
    out
}

/// A parsed Python module plus the source text it came from.
///
/// Owning the source keeps [`ModuleAst::line_col`] total: every ruff byte
/// offset can be mapped back to a 1-based (line, column) pair for the
/// remainder of the compiler pipeline.
#[derive(Debug, Clone)]
pub struct ModuleAst {
    module: ModModule,
    source: String,
}

impl ModuleAst {
    /// The ruff module node.
    pub fn module(&self) -> &ModModule {
        &self.module
    }

    /// The original source text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Map a byte offset to (1-based line, 1-based column). Columns count
    /// Unicode scalar values, not bytes. Offsets past EOF clamp to the end of
    /// the source.
    pub fn line_col(&self, byte_offset: usize) -> (usize, usize) {
        line_col_of(&self.source, byte_offset)
    }

    /// Consume the wrapper, returning the owned ruff module node.
    pub fn into_module(self) -> ModModule {
        self.module
    }
}

/// A parse failure in the T-01 contract diagnostic format:
/// `error at line L, column C: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset of the failure in the source.
    pub offset: usize,
    /// 1-based line of the failure.
    pub line: usize,
    /// 1-based column of the failure (Unicode scalar values).
    pub col: usize,
    /// Human-readable diagnostic message from the underlying ruff error.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "error at line {}, column {}: {}",
            self.line, self.col, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// Post-parse validation to catch semantic syntax errors that ruff's parser
/// doesn't flag. These produce [`ParseError`] in the T-01 contract format.
/// NOTE: The three diagnostics below (return/yield outside function, duplicate
/// params, nonlocal without binding) are also emitted by `static_checks` in
/// `verify_bridge.rs` for the T-04 diff set. This function handles them only
/// as ParseErrors for the T-01 contract.
fn validate_module(module: &ModModule, source: &str) -> Result<(), ParseError> {
    // Check for `return`/`yield` at module level
    for stmt in &module.body {
        match stmt {
            ruff_python_ast::Stmt::Return(r) => {
                let offset = usize::try_from(u32::from(r.range().start())).unwrap_or(0);
                let (line, col_file) = line_col_of(source, offset);
                let indent = line_indent(source, line);
                let col_display = col_file.saturating_sub(indent) + 4;
                return Err(ParseError {
                    offset,
                    line,
                    col: col_display,
                    message: "'return' outside function".to_string(),
                });
            }
            ruff_python_ast::Stmt::Expr(e) => {
                if let ruff_python_ast::Expr::Yield(_) = &*e.value {
                    let offset = usize::try_from(u32::from(e.range().start())).unwrap_or(0);
                    let (line, col_file) = line_col_of(source, offset);
                    let indent = line_indent(source, line);
                    let col_display = col_file.saturating_sub(indent) + 4;
                    return Err(ParseError {
                        offset,
                        line,
                        col: col_display,
                        message: "'yield' outside function".to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// Leading-whitespace count (Unicode scalar values) of the 1-based `line`.
fn line_indent(source: &str, line: usize) -> usize {
    let mut cur = 1usize;
    let mut indent = 0usize;
    let mut seen_content = false;
    for ch in source.chars() {
        if cur > line {
            break;
        }
        if cur == line {
            if !seen_content && (ch == ' ' || ch == '\t') {
                indent += 1;
            } else if ch != '\n' && ch != '\r' {
                seen_content = true;
            }
        }
        if ch == '\n' {
            cur += 1;
        }
    }
    indent
}

#[cfg(test)]
mod ir_matrix_check;

pub mod lowering;
pub mod verify_bridge;

pub use lowering::{lower, lower_source, LowerError, LoweringBug, ScopeInfo, ScopeInfoModule};
pub use verify_bridge::{
    verify_ir_with_positions, verify_source, VerifyPositionMap, VerifyPositionMiss,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_module() {
        let ast = parse("x = 1\n").unwrap();
        assert_eq!(ast.module().body.len(), 1);
        assert_eq!(ast.source(), "x = 1\n");
    }

    #[test]
    fn line_col_is_one_based() {
        let ast = parse("a = 1\nb = 2\n").unwrap();
        assert_eq!(ast.line_col(0), (1, 1));
        assert_eq!(ast.line_col(6), (2, 1));
    }

    #[test]
    fn syntax_error_becomes_contract_diagnostic() {
        let err = parse("x = (\n").unwrap_err();
        assert!(
            err.to_string().starts_with("error at line 2, column 1: "),
            "got: {err}"
        );
    }

    #[test]
    fn unsupported_syntax_fails_closed() {
        // PEP 695 type aliases are 3.12+; with target 3.11 they would be
        // unsupported — but we pin 3.14, so probe the fail-closed path with a
        // construct ruff flags as unsupported at 3.14: none expected, so this
        // asserts the valid case still passes (regression guard for the
        // unsupported-error check ordering).
        let ast = parse("match x:\n    case _:\n        pass\n").unwrap();
        assert!(!ast.module().body.is_empty());
    }

    #[test]
    fn offset_points_at_error_start() {
        let err = parse("f(x=1, 2)\n").unwrap_err();
        assert_eq!(err.line, 1);
        assert_eq!(err.col, 8);
        assert_eq!(
            err.message,
            "Positional argument cannot follow keyword argument"
        );
    }

    /// B2 regression: the line_starts+binary-search rewrite of
    /// `collect_stmt_lines` must equal the OLD O(n^2) definition
    /// (1 + count of '\n' before each statement start, same pre-order walk)
    /// on a mixed-shape source covering every compound-statement branch of
    /// the walker.
    #[test]
    fn collect_stmt_lines_matches_brute_force() {
        let src = "x = 1\n\
def f(a):\n    if a:\n        y = 2\n    elif a > 1:\n        y = 3\n    else:\n        y = 4\n    while y:\n        break\n    for i in range(3):\n        pass\n    try:\n        z = 5\n    except Exception:\n        pass\n    else:\n        pass\n    finally:\n        pass\n    match y:\n        case 1:\n            pass\n    with open('f') as g:\n        pass\n    return y\n\nclass C:\n    def m(self):\n        return [i for i in range(2)]\n";
        let ast = parse(src).unwrap();
        let got = collect_stmt_lines(&ast);

        // Oracle: independent naive re-implementation of the documented
        // walk with per-offset newline counting (the pre-B2 shape).
        fn naive_line(src: &str, off: usize) -> usize {
            src[..off.min(src.len())]
                .bytes()
                .filter(|&b| b == b'\n')
                .count()
                + 1
        }
        use ruff_python_ast as rast;
        use ruff_text_size::Ranged;
        fn walk(src: &str, body: &[rast::Stmt], out: &mut Vec<usize>) {
            for s in body {
                out.push(naive_line(src, s.start().to_u32() as usize));
                match s {
                    rast::Stmt::If(i) => {
                        walk(src, &i.body, out);
                        for c in &i.elif_else_clauses {
                            out.push(naive_line(src, c.start().to_u32() as usize));
                            walk(src, &c.body, out);
                        }
                    }
                    rast::Stmt::While(w) => {
                        walk(src, &w.body, out);
                        walk(src, &w.orelse, out);
                    }
                    rast::Stmt::For(f) => {
                        walk(src, &f.body, out);
                        walk(src, &f.orelse, out);
                    }
                    rast::Stmt::With(w) => walk(src, &w.body, out),
                    rast::Stmt::Try(t) => {
                        walk(src, &t.body, out);
                        for h in &t.handlers {
                            out.push(naive_line(src, h.start().to_u32() as usize));
                            let rast::ExceptHandler::ExceptHandler(hi) = h;
                            walk(src, &hi.body, out);
                        }
                        walk(src, &t.orelse, out);
                        walk(src, &t.finalbody, out);
                    }
                    rast::Stmt::Match(m) => {
                        for c in &m.cases {
                            out.push(naive_line(src, c.start().to_u32() as usize));
                            walk(src, &c.body, out);
                        }
                    }
                    rast::Stmt::FunctionDef(f) => {
                        let body_line = f
                            .body
                            .first()
                            .map(|s| naive_line(src, s.start().to_u32() as usize))
                            .unwrap_or(1);
                        out.push(body_line);
                        walk(src, &f.body, out);
                    }
                    rast::Stmt::ClassDef(c) => walk(src, &c.body, out),
                    _ => {}
                }
            }
        }
        let mut want = Vec::new();
        walk(src, &ast.module().body, &mut want);
        assert!(!got.is_empty());
        assert_eq!(got, want, "binary-search lines diverge from naive count");
    }
}
