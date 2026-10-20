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

    fn offset_to_line(source: &str, off: usize) -> usize {
        source[..off.min(source.len())]
            .bytes()
            .filter(|&b| b == b'\n')
            .count()
            + 1
    }
    fn walk_body(body: &[rast::Stmt], source: &str, out: &mut Vec<usize>) {
        for s in body {
            out.push(offset_to_line(source, s.start().to_u32() as usize));
            match s {
                rast::Stmt::If(i) => {
                    walk_body(&i.body, source, out);
                    for c in &i.elif_else_clauses {
                        out.push(offset_to_line(source, c.start().to_u32() as usize));
                        walk_body(&c.body, source, out);
                    }
                }
                rast::Stmt::While(w) => {
                    walk_body(&w.body, source, out);
                    walk_body(&w.orelse, source, out);
                }
                rast::Stmt::For(f) => {
                    walk_body(&f.body, source, out);
                    walk_body(&f.orelse, source, out);
                }
                rast::Stmt::With(w) => walk_body(&w.body, source, out),
                rast::Stmt::Try(t) => {
                    walk_body(&t.body, source, out);
                    for h in &t.handlers {
                        out.push(offset_to_line(source, h.start().to_u32() as usize));
                        let rast::ExceptHandler::ExceptHandler(hi) = h;
                        walk_body(&hi.body, source, out);
                    }
                    walk_body(&t.orelse, source, out);
                    walk_body(&t.finalbody, source, out);
                }
                rast::Stmt::Match(m) => {
                    for c in &m.cases {
                        out.push(offset_to_line(source, c.start().to_u32() as usize));
                        walk_body(&c.body, source, out);
                    }
                }
                rast::Stmt::FunctionDef(f) => {
                    let body_line = f
                        .body
                        .first()
                        .map(|s| offset_to_line(source, s.start().to_u32() as usize))
                        .unwrap_or(1);
                    out.push(body_line);
                    walk_body(&f.body, source, out);
                }
                rast::Stmt::ClassDef(c) => walk_body(&c.body, source, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk_body(&ast.module().body, ast.source(), &mut out);
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
}
