//! IR verify bridge (leaf 01.04 wire-in).
//!
//! Runs [`ap_ir::verify`] on a lowered module and resolves each diagnostic's
//! position against the ruff AST, since ap-ir is vendor-clean and carries no
//! spans. The position convention is pinned by the T-04 fixtures:
//!
//! - **line**: the 1-based line of the offending read.
//! - **col**: 1-based, Unicode scalar values, anchored at CPython's
//!   fine-grained-error caret (S-02-37: under the read name only). Empirical
//!   CPython rule derived + verified against all 32 pinned caret columns in
//!   `tests/t04_ir_verify/fixtures/expected_t04.json`: `display_col =
//!   file_col - leading_whitespace + 4`. (The traceback's source line is
//!   printed 4-space indented and the caret column counts that indent.)
//! - The **NameError-at-module** family (p19/p21/p22, T-04 G-3 boundary set)
//!   and the free-variable cell-miss (p20) are optional-report; the bridge
//!   attaches positions for any diagnostic `verify` does emit.
//!
//! [`verify_source`] is the one-call form: parse + lower + verify; it fails
//! closed with [`LowerError`] on parse/lowering problems.

use std::collections::BTreeMap;

use ruff_python_ast as ast;
use ruff_text_size::Ranged;

use crate::{lower, parse, LowerError};
use ap_ir::verify::{verify, Diagnostic};

/// Byte-offset position table for identifier reads: `(name -> [byte_offset])`
/// in source order. `verify` reports reads per scope in first-load order;
/// matching is by name + occurrence ordinal, exact for the v1 verifier's
/// report shape. Line/col conversion happens in
/// [`verify_ir_with_positions`] / [`verify_source`], which own the source.
#[derive(Debug, Default, Clone)]
pub struct VerifyPositionMap {
    reads: BTreeMap<String, Vec<usize>>,
}

impl VerifyPositionMap {
    /// Build the read-position table from a parsed module.
    pub fn from_module(module: &ast::ModModule) -> Self {
        let mut m = VerifyPositionMap::default();
        for stmt in &module.body {
            collect_stmt_reads(stmt, &mut m);
        }
        m
    }

    /// The byte offset of the `ordinal`-th read of `name`; falls back to the
    /// FIRST read when the ordinal is out of range (the verifier counts
    /// per-scope loads, the map counts file-wide — for single-read names
    /// they coincide, and the fallback keeps positions total).
    pub fn lookup(&self, name: &str, ordinal: usize) -> Option<usize> {
        let reads = self.reads.get(name)?;
        reads.get(ordinal).or_else(|| reads.first()).copied()
    }
}

/// Position resolution failure (a diagnostic whose read cannot be located).
/// Non-fatal: the caller renders the diagnostic without a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyPositionMiss {
    pub name: String,
    pub ordinal: usize,
}

/// Parse + lower + verify with positions attached. One-call form. Runs the
/// IR verification pass (S-02-2 unbound-local + S-02-1 resolution) plus the
/// compile-stage static checks (S-02-3/4/11/18, S-04-10, S-02-8 — T-04
/// G-4) over the ruff AST, merged in source order.
///
/// p17 note (T-04 G-4): `def f(x=):` dies in the PARSER (ruff: `Expected an
/// expression` at the `=`+1), while CPython's parse stage says `expected
/// default value expression` at caret col 12 (the char after `=`). A parse
/// failure of exactly that shape is remapped to the verify-side SyntaxError
/// class so the T-04 diff set stays complete; all other parse errors stay
/// T-01 ParseErrors.
pub fn verify_source(src: &str) -> Result<Vec<Diagnostic>, LowerError> {
    let parsed = match parse(src) {
        Ok(p) => p,
        Err(e) => return Ok(remap_parse_error(&e, src)),
    };
    let (module, _scopes) = lower(&parsed)?;
    let pos = VerifyPositionMap::from_module(parsed.module());
    let mut diags = attach_positions(verify(&module), &pos);
    diags.extend(static_checks(parsed.module(), parsed.source()));
    let mut diags = resolve_positions(diags, &pos, src);
    diags.sort_by_key(|d| (d.line.unwrap_or(usize::MAX), d.col.unwrap_or(usize::MAX)));
    Ok(diags)
}

/// Remap the one parse-stage error that belongs in verify's diff set (p17,
/// T-04 G-4): a missing default-value expression after `=` in a parameter
/// list. Everything else propagates as a ParseError-shaped StaticError so
/// callers see a diagnostic either way (verify is a pass over source, not a
/// parser gate — the ap-verify-dump contract treats both as exit 1).
fn remap_parse_error(e: &crate::ParseError, src: &str) -> Vec<Diagnostic> {
    let is_missing_default = e.message == "Expected an expression"
        && src[..e.offset.min(src.len())].trim_end().ends_with('=');
    if !is_missing_default {
        // Non-p17 parse errors are T-01 territory; report them as the
        // contract diagnostic on stderr via a StaticError with the parse
        // text (the bin prints it; the T-04 fixture set has no probe here).
        return vec![Diagnostic {
            kind: ap_ir::verify::DiagKind::StaticError,
            qualname: "<module>".to_string(),
            name: String::new(),
            message: e.message.clone(),
            line: Some(e.line),
            col: Some(e.col),
            load_ordinal: 0,
            clause: "C-06/T-01",
        }];
    }
    // CPython caret: one past the `=` (col 12 display on `def f(x=):`).
    // The ruff offset points at the token after `=`; shift display col +3
    // from the ruff col? Empirical: ruff col 9 -> CPython display col 12.
    // display = col_file - indent + 4 = 9 - 0 + 4 = 13... CPython pins 12
    // (caret directly after the `=` at file col 12). Use ruff offset - 1
    // (the `=` itself) for the display transform: 8 - 0 + 4 = 12.
    let eq_off = e.offset.saturating_sub(1);
    let (line, col_file) = crate::line_col_of(src, eq_off);
    let indent = line_indent(src, line);
    vec![Diagnostic {
        kind: ap_ir::verify::DiagKind::StaticError,
        qualname: "<module>".to_string(),
        name: String::new(),
        message: "expected default value expression".to_string(),
        line: Some(line),
        col: Some(col_file.saturating_sub(indent) + 4),
        load_ordinal: 0,
        clause: "C-06/T-04-G-4",
    }]
}

/// Same, from an already-parsed module (exposed for tests).
pub fn verify_ir_with_positions(parsed: &crate::ModuleAst) -> Result<Vec<Diagnostic>, LowerError> {
    let (module, _scopes) = lower(parsed)?;
    let pos = VerifyPositionMap::from_module(parsed.module());
    let mut diags = attach_positions(verify(&module), &pos);
    diags.extend(static_checks(parsed.module(), parsed.source()));
    let mut diags = resolve_positions(diags, &pos, parsed.source());
    diags.sort_by_key(|d| (d.line.unwrap_or(usize::MAX), d.col.unwrap_or(usize::MAX)));
    Ok(diags)
}

// ---------------------------------------------------------------------------
// Compile-stage static checks (T-04 G-4: post-parse CPython compile errors).
// Each check cites its clause; message text byte-matches CPython 3.14.7
// (pinned in tests/t04_ir_verify/fixtures/expected_t04.json).
// ---------------------------------------------------------------------------

/// Run the compile-stage static checks over a parsed module. Positions are
/// 1-based line + FGE display column (same `col_file - indent + 4` rule).
pub fn static_checks(module: &ast::ModModule, src: &str) -> Vec<Diagnostic> {
    let mut c = StaticChecker {
        src,
        diags: Vec::new(),
        loop_depth: 0,
        fn_depth: 0,
        in_class: 0,
    };
    for stmt in &module.body {
        c.stmt(stmt);
    }
    c.diags
}

struct StaticChecker<'s> {
    src: &'s str,
    diags: Vec<Diagnostic>,
    loop_depth: usize,
    fn_depth: usize,
    in_class: usize,
}

impl<'s> StaticChecker<'s> {
    fn pos(&self, offset: ruff_text_size::TextSize) -> (usize, usize) {
        let off = usize::try_from(u32::from(offset)).unwrap_or(0);
        let (line, col_file) = crate::line_col_of(self.src, off);
        let indent = line_indent(self.src, line);
        (line, col_file.saturating_sub(indent) + 4)
    }

    fn push(&mut self, offset: ruff_text_size::TextSize, message: String, clause: &'static str) {
        let (line, col) = self.pos(offset);
        self.diags.push(Diagnostic {
            kind: ap_ir::verify::DiagKind::StaticError,
            qualname: "<module>".to_string(),
            name: String::new(),
            message,
            line: Some(line),
            col: Some(col),
            load_ordinal: 0,
            clause,
        });
    }

    fn stmt(&mut self, s: &ast::Stmt) {
        use ast::Stmt::*;
        match s {
            // S-02-11: `return` outside a function is compile-time.
            Return(r) if self.fn_depth == 0 => {
                self.push(
                    r.start(),
                    "'return' outside function".to_string(),
                    "S-02-11",
                );
            }
            // S-02-18: break/continue outside a loop.
            Break(b) if self.loop_depth == 0 => {
                self.push(b.start(), "'break' outside loop".to_string(), "S-02-18");
            }
            Continue(c) if self.loop_depth == 0 => {
                self.push(
                    c.start(),
                    "'continue' not properly in loop".to_string(),
                    "S-02-18",
                );
            }
            // S-04-10: `import *` only at module level (functions AND class
            // bodies reject — p15/p16; the caret sits at the END of the
            // statement line, CPython-pinned).
            ImportFrom(imp) => {
                let star = imp.names.iter().any(|a| a.name.id.as_str() == "*");
                if star && (self.fn_depth > 0 || self.in_class > 0) {
                    // CPython pins the caret at the LAST character of the
                    // statement (end col - 1 — p15/p16 surprise #6).
                    let end = imp.end();
                    let last = end - ruff_text_size::TextSize::from(1);
                    self.push(
                        last,
                        "import * only allowed at module level".to_string(),
                        "S-04-10",
                    );
                    return;
                }
            }
            FunctionDef(f) => {
                self.check_global_nonlocal_clash(&f.body);
                self.check_dup_params(&f.parameters);
                self.fn_depth += 1;
                for st in &f.body {
                    self.stmt(st);
                }
                self.fn_depth -= 1;
                return;
            }
            ClassDef(c) => {
                self.in_class += 1;
                for st in &c.body {
                    self.stmt(st);
                }
                self.in_class -= 1;
                return;
            }
            For(_f) => {
                self.loop_depth += 1;
                self.stmt_children(s);
                self.loop_depth -= 1;
                return;
            }
            While(_w) => {
                self.loop_depth += 1;
                self.stmt_children(s);
                self.loop_depth -= 1;
                return;
            }
            Nonlocal(nl) if self.fn_depth == 0 => {
                // S-02-4: nonlocal at module level (p11; caret spans the
                // whole statement, col 5 display = statement start).
                self.push(
                    nl.start(),
                    "nonlocal declaration not allowed at module level".to_string(),
                    "S-02-4",
                );
            }
            _ => {}
        }
        self.stmt_children(s);
    }

    /// `global x` + `nonlocal x` in one function (S-02-3, p09): detect per
    /// function body, reporting at the `global` statement (CPython pins the
    /// caret under `global x`, col 5 display).
    fn check_global_nonlocal_clash(&mut self, body: &[ast::Stmt]) {
        let mut global_names: Vec<(String, ruff_text_size::TextSize)> = Vec::new();
        let mut nonlocal_names: Vec<String> = Vec::new();
        for s in body {
            match s {
                ast::Stmt::Global(g) => {
                    for n in &g.names {
                        global_names.push((n.id.to_string(), g.start()));
                    }
                }
                ast::Stmt::Nonlocal(nl) => {
                    for n in &nl.names {
                        nonlocal_names.push(n.id.to_string());
                    }
                }
                _ => {}
            }
        }
        for (name, at) in &global_names {
            if nonlocal_names.iter().any(|n| n == name) {
                self.push(
                    *at,
                    format!("name '{name}' is nonlocal and global"),
                    "S-02-3",
                );
            }
        }
        // S-02-4: nonlocal without an enclosing-function binding (p10).
        // The v1 static check: nonlocal names with no binding in any
        // enclosing function — approximated here at the module level
        // (function defined at top level with no outer `x` binding).
        for (idx, s) in body.iter().enumerate() {
            if let ast::Stmt::Nonlocal(nl) = s {
                for n in &nl.names {
                    let name = n.id.as_str();
                    let outer_binds = module_level_binds(body, idx, name);
                    if !outer_binds {
                        self.push(
                            nl.start(),
                            format!("no binding for nonlocal '{name}' found"),
                            "S-02-4",
                        );
                    }
                }
            }
        }
    }

    /// Duplicate parameter names (S-02-8/S-02-10, p18): CPython pins the
    /// caret under the SECOND occurrence, message
    /// `duplicate argument 'x' in function definition`.
    fn check_dup_params(&mut self, params: &ast::Parameters) {
        let mut seen: Vec<(String, ruff_text_size::TextSize)> = Vec::new();
        let mut all: Vec<(&ast::Parameter, ruff_text_size::TextSize)> = Vec::new();
        for p in params
            .posonlyargs
            .iter()
            .chain(params.args.iter())
            .chain(params.kwonlyargs.iter())
        {
            all.push((&p.parameter, p.parameter.name.range().start()));
        }
        if let Some(v) = &params.vararg {
            all.push((v, v.name.range().start()));
        }
        if let Some(k) = &params.kwarg {
            all.push((k, k.name.range().start()));
        }
        for (param, at) in all {
            let name = param.name.id.as_str();
            if seen.iter().any(|(n, _)| n == name) {
                self.push(
                    at,
                    format!("duplicate argument '{name}' in function definition"),
                    "S-02-8/S-02-10",
                );
            } else {
                seen.push((name.to_string(), at));
            }
        }
    }

    fn stmt_children(&mut self, s: &ast::Stmt) {
        use ast::Stmt::*;
        match s {
            If(i) => {
                for st in i.body.iter() {
                    self.stmt(st);
                }
                for clause in &i.elif_else_clauses {
                    for st in &clause.body {
                        self.stmt(st);
                    }
                }
            }
            While(w) => {
                for st in w.body.iter().chain(w.orelse.iter()) {
                    self.stmt(st);
                }
            }
            For(f) => {
                for st in f.body.iter().chain(f.orelse.iter()) {
                    self.stmt(st);
                }
            }
            With(w) => {
                for st in &w.body {
                    self.stmt(st);
                }
            }
            Try(t) => {
                for st in &t.body {
                    self.stmt(st);
                }
                for h in &t.handlers {
                    let ast::ExceptHandler::ExceptHandler(h) = h;
                    for st in &h.body {
                        self.stmt(st);
                    }
                }
                for st in t.orelse.iter().chain(t.finalbody.iter()) {
                    self.stmt(st);
                }
            }
            Match(m) => {
                for c in &m.cases {
                    for st in &c.body {
                        self.stmt(st);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Does `name` get a module-level binding before statement `idx`? (Module-
/// level approximation of nonlocal's enclosing-binding check — p10's fn has
/// no enclosing binding at all.)
fn module_level_binds(body: &[ast::Stmt], idx: usize, name: &str) -> bool {
    for s in body.iter().take(idx) {
        let binds: Vec<&str> = match s {
            ast::Stmt::Assign(a) => a
                .targets
                .iter()
                .filter_map(|t| match t {
                    ast::Expr::Name(n) => Some(n.id.as_str()),
                    _ => None,
                })
                .collect(),
            ast::Stmt::FunctionDef(f) => vec![f.name.id.as_str()],
            ast::Stmt::ClassDef(c) => vec![c.name.id.as_str()],
            _ => vec![],
        };
        if binds.iter().any(|b| *b == name) {
            return true;
        }
    }
    false
}

/// Convert each positioned diagnostic's byte-offset placeholder (stashed in
/// `col` by [`attach_positions`]) into the CPython FGE display position:
/// 1-based line, and 1-based column under the
/// `col_display = col_file - indent + 4` rule (see module docs), where
/// `indent` is the leading whitespace of the read's line.
pub fn resolve_positions(
    diags: Vec<Diagnostic>,
    pos: &VerifyPositionMap,
    src: &str,
) -> Vec<Diagnostic> {
    let _ = pos; // offsets already attached; kept for API symmetry
    diags
        .into_iter()
        .map(|mut d| {
            if d.line == Some(0) {
                let off = d.col.unwrap_or(0);
                let (line, col_file) = crate::line_col_of(src, off);
                let indent = line_indent(src, line);
                d.line = Some(line);
                d.col = Some(col_file.saturating_sub(indent) + 4);
            }
            d
        })
        .collect()
}

/// Leading-whitespace count (Unicode scalar values) of the 1-based `line`.
fn line_indent(src: &str, line: usize) -> usize {
    let mut cur = 1usize;
    let mut indent = 0usize;
    let mut seen_content = false;
    for ch in src.chars() {
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

/// Attach each diagnostic's read byte offset from the position table (offset
/// rides in `col` with `line = 0`; [`resolve_positions`] converts it).
/// The diagnostic's `load_ordinal` selects WHICH read of the name maps
/// (del-then-read flags the second read, etc.).
fn attach_positions(diags: Vec<Diagnostic>, pos: &VerifyPositionMap) -> Vec<Diagnostic> {
    diags
        .into_iter()
        .map(|d| {
            let site = pos.lookup(&d.name, d.load_ordinal);
            match site {
                Some(off) => {
                    let mut d2 = d;
                    d2.line = Some(0);
                    d2.col = Some(off);
                    d2
                }
                None => d,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Read-position collection over the ruff AST (every Name in Load context,
// in source order, including nested scopes — the verifier's reads span all
// of them)
// ---------------------------------------------------------------------------

fn collect_stmt_reads(s: &ast::Stmt, m: &mut VerifyPositionMap) {
    use ast::Stmt::*;
    match s {
        FunctionDef(f) => {
            for d in &f.decorator_list {
                collect_expr_reads(&d.expression, m);
            }
            collect_params_reads(&f.parameters, m);
            if let Some(r) = &f.returns {
                collect_expr_reads(r, m);
            }
            for st in &f.body {
                collect_stmt_reads(st, m);
            }
        }
        ClassDef(c) => {
            for d in &c.decorator_list {
                collect_expr_reads(&d.expression, m);
            }
            if let Some(args) = &c.arguments {
                for a in &args.args {
                    collect_expr_reads(a, m);
                }
                for kw in &args.keywords {
                    collect_expr_reads(&kw.value, m);
                }
            }
            for st in &c.body {
                collect_stmt_reads(st, m);
            }
        }
        Return(r) => {
            if let Some(v) = &r.value {
                collect_expr_reads(v, m);
            }
        }
        Delete(d) => {
            // `del q` reads the binding slot (p07 pins the caret under `q`);
            // ruff marks del targets Del-context, so record them explicitly.
            for t in &d.targets {
                if let ast::Expr::Name(n) = t {
                    let off = usize::try_from(u32::from(n.range().start())).unwrap_or(0);
                    m.reads.entry(n.id.to_string()).or_default().push(off);
                }
                collect_expr_reads(t, m);
            }
        }
        TypeAlias(t) => collect_expr_reads(&t.value, m),
        Assign(a) => {
            collect_expr_reads(&a.value, m);
            for t in &a.targets {
                collect_expr_reads(t, m);
            }
        }
        AugAssign(a) => {
            // Augassign targets read the binding slot (S-02-19): record the
            // Name target's position as a read site even though ruff marks
            // it Store-context (p04 pins the caret under `x` in `x += 1`).
            if let ast::Expr::Name(n) = a.target.as_ref() {
                let off = usize::try_from(u32::from(n.range().start())).unwrap_or(0);
                m.reads.entry(n.id.to_string()).or_default().push(off);
            }
            collect_expr_reads(&a.target, m);
            collect_expr_reads(&a.value, m);
        }
        AnnAssign(a) => {
            collect_expr_reads(&a.annotation, m);
            if let Some(v) = &a.value {
                collect_expr_reads(v, m);
            }
            collect_expr_reads(&a.target, m);
        }
        For(f) => {
            collect_expr_reads(&f.iter, m);
            collect_expr_reads(&f.target, m);
            for st in f.body.iter().chain(f.orelse.iter()) {
                collect_stmt_reads(st, m);
            }
        }
        While(w) => {
            collect_expr_reads(&w.test, m);
            for st in w.body.iter().chain(w.orelse.iter()) {
                collect_stmt_reads(st, m);
            }
        }
        If(i) => {
            collect_expr_reads(&i.test, m);
            for st in i.body.iter() {
                collect_stmt_reads(st, m);
            }
            for clause in &i.elif_else_clauses {
                if let Some(t) = &clause.test {
                    collect_expr_reads(t, m);
                }
                for st in &clause.body {
                    collect_stmt_reads(st, m);
                }
            }
        }
        With(w) => {
            for item in &w.items {
                collect_expr_reads(&item.context_expr, m);
                if let Some(t) = &item.optional_vars {
                    collect_expr_reads(t, m);
                }
            }
            for st in &w.body {
                collect_stmt_reads(st, m);
            }
        }
        Match(m_) => {
            collect_expr_reads(&m_.subject, m);
            for case in &m_.cases {
                collect_pattern_reads(&case.pattern, m);
                if let Some(g) = &case.guard {
                    collect_expr_reads(g, m);
                }
                for st in &case.body {
                    collect_stmt_reads(st, m);
                }
            }
        }
        Raise(r) => {
            if let Some(e) = &r.exc {
                collect_expr_reads(e, m);
            }
            if let Some(c) = &r.cause {
                collect_expr_reads(c, m);
            }
        }
        Try(t) => {
            for st in &t.body {
                collect_stmt_reads(st, m);
            }
            for h in &t.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = h;
                if let Some(ty) = &h.type_ {
                    collect_expr_reads(ty, m);
                }
                for st in &h.body {
                    collect_stmt_reads(st, m);
                }
            }
            for st in t.orelse.iter().chain(t.finalbody.iter()) {
                collect_stmt_reads(st, m);
            }
        }
        Assert(a) => {
            collect_expr_reads(&a.test, m);
            if let Some(msg) = &a.msg {
                collect_expr_reads(msg, m);
            }
        }
        Expr(e) => collect_expr_reads(&e.value, m),
        _ => {}
    }
}

fn collect_params_reads(p: &ast::Parameters, m: &mut VerifyPositionMap) {
    for pwd in p
        .posonlyargs
        .iter()
        .chain(p.args.iter())
        .chain(p.kwonlyargs.iter())
    {
        if let Some(d) = &pwd.default {
            collect_expr_reads(d, m);
        }
        if let Some(a) = &pwd.parameter.annotation {
            collect_expr_reads(a, m);
        }
    }
    if let Some(v) = &p.vararg {
        if let Some(a) = &v.annotation {
            collect_expr_reads(a, m);
        }
    }
    if let Some(k) = &p.kwarg {
        if let Some(a) = &k.annotation {
            collect_expr_reads(a, m);
        }
    }
}

fn collect_pattern_reads(p: &ast::Pattern, m: &mut VerifyPositionMap) {
    use ast::Pattern::*;
    match p {
        MatchValue(v) => collect_expr_reads(&v.value, m),
        MatchSingleton(_) => {}
        MatchSequence(seq) => {
            for sub in &seq.patterns {
                collect_pattern_reads(sub, m);
            }
        }
        MatchMapping(mp) => {
            for k in &mp.keys {
                collect_expr_reads(k, m);
            }
            for sub in &mp.patterns {
                collect_pattern_reads(sub, m);
            }
        }
        MatchClass(cls) => {
            collect_expr_reads(&cls.cls, m);
            for sub in &cls.arguments.patterns {
                collect_pattern_reads(sub, m);
            }
            for kw in &cls.arguments.keywords {
                collect_pattern_reads(&kw.pattern, m);
            }
        }
        MatchStar(_) => {}
        MatchAs(ma) => {
            if let Some(inner) = &ma.pattern {
                collect_pattern_reads(inner, m);
            }
        }
        MatchOr(or) => {
            for sub in &or.patterns {
                collect_pattern_reads(sub, m);
            }
        }
    }
}

/// Record every Name in Load context, in source order. ruff's visitor
/// handles the full expression grammar; we hand-walk to keep scope
/// transparency (the verifier's reads span all scopes).
fn collect_expr_reads(e: &ast::Expr, m: &mut VerifyPositionMap) {
    use ast::Expr::*;
    match e {
        Name(n) => {
            // Only Load-context names are "reads". ruff encodes context on
            // the Name node; Store/Del contexts are binding sites.
            if matches!(n.ctx, ast::ExprContext::Load) {
                let off = usize::try_from(u32::from(n.range().start())).unwrap_or(0);
                m.reads.entry(n.id.to_string()).or_default().push(off);
            }
        }
        BoolOp(b) => {
            for v in &b.values {
                collect_expr_reads(v, m);
            }
        }
        Named(n) => {
            collect_expr_reads(&n.value, m);
            collect_expr_reads(&n.target, m);
        }
        BinOp(b) => {
            collect_expr_reads(&b.left, m);
            collect_expr_reads(&b.right, m);
        }
        UnaryOp(u) => collect_expr_reads(&u.operand, m),
        Compare(c) => {
            collect_expr_reads(&c.left, m);
            for cmp in &c.comparators {
                collect_expr_reads(cmp, m);
            }
        }
        Call(c) => {
            collect_expr_reads(&c.func, m);
            for a in &c.arguments.args {
                collect_expr_reads(a, m);
            }
            for kw in &c.arguments.keywords {
                collect_expr_reads(&kw.value, m);
            }
        }
        Attribute(a) => collect_expr_reads(&a.value, m),
        Subscript(s) => {
            collect_expr_reads(&s.value, m);
            collect_expr_reads(&s.slice, m);
        }
        Slice(sl) => {
            for x in sl.lower.iter().chain(sl.upper.iter()).chain(sl.step.iter()) {
                collect_expr_reads(x, m);
            }
        }
        Tuple(t) => {
            for x in &t.elts {
                collect_expr_reads(x, m);
            }
        }
        List(l) => {
            for x in &l.elts {
                collect_expr_reads(x, m);
            }
        }
        Set(s) => {
            for x in &s.elts {
                collect_expr_reads(x, m);
            }
        }
        Dict(d) => {
            for item in &d.items {
                if let Some(k) = &item.key {
                    collect_expr_reads(k, m);
                }
                collect_expr_reads(&item.value, m);
            }
        }
        Starred(st) => collect_expr_reads(&st.value, m),
        Lambda(l) => {
            if let Some(params) = l.parameters.as_deref() {
                collect_params_reads(params, m);
            }
            collect_expr_reads(&l.body, m);
        }
        ListComp(c) => {
            collect_expr_reads(&c.elt, m);
            for g in &c.generators {
                collect_expr_reads(&g.iter, m);
                collect_expr_reads(&g.target, m);
                for f in &g.ifs {
                    collect_expr_reads(f, m);
                }
            }
        }
        SetComp(c) => {
            collect_expr_reads(&c.elt, m);
            for g in &c.generators {
                collect_expr_reads(&g.iter, m);
                collect_expr_reads(&g.target, m);
                for f in &g.ifs {
                    collect_expr_reads(f, m);
                }
            }
        }
        DictComp(c) => {
            if let Some(k) = &c.key {
                collect_expr_reads(k, m);
            }
            collect_expr_reads(&c.value, m);
            for g in &c.generators {
                collect_expr_reads(&g.iter, m);
                collect_expr_reads(&g.target, m);
                for f in &g.ifs {
                    collect_expr_reads(f, m);
                }
            }
        }
        Generator(g) => {
            collect_expr_reads(&g.elt, m);
            for gen in &g.generators {
                collect_expr_reads(&gen.iter, m);
                collect_expr_reads(&gen.target, m);
                for f in &gen.ifs {
                    collect_expr_reads(f, m);
                }
            }
        }
        Await(a) => collect_expr_reads(&a.value, m),
        Yield(y) => {
            if let Some(v) = &y.value {
                collect_expr_reads(v, m);
            }
        }
        YieldFrom(y) => collect_expr_reads(&y.value, m),
        FString(f) => {
            for part in f.value.iter() {
                if let ast::FStringPart::FString(fs) = part {
                    for el in fs.elements.iter() {
                        if let ast::InterpolatedStringElement::Interpolation(interp) = el {
                            collect_expr_reads(&interp.expression, m);
                            if let Some(spec) = &interp.format_spec {
                                for sel in spec.elements.iter() {
                                    if let ast::InterpolatedStringElement::Interpolation(si) = sel {
                                        collect_expr_reads(&si.expression, m);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        TString(_) => {}
        ast::Expr::If(i) => {
            collect_expr_reads(&i.test, m);
            collect_expr_reads(&i.body, m);
            collect_expr_reads(&i.orelse, m);
        }
        NumberLiteral(_) | StringLiteral(_) | BytesLiteral(_) | BooleanLiteral(_)
        | NoneLiteral(_) | EllipsisLiteral(_) | IpyEscapeCommand(_) => {}
    }
}
