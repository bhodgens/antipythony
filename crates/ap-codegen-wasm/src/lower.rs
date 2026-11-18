//! ap-ir → WasmGC module lowering (leaves 03.02a/b/c: values + control
//! flow + calls + closures).
//!
//! Emits the spec-07 module skeleton (S-07-43): type section (one rec
//! group, `types.rs`), WASI p1 + `aprt` imports (S-07-41), singleton
//! globals, memory + literal data sections, `rt_init`, the EMITTED-wasm
//! prelude runtime (`prelude.rs`), the per-FnDef user-function bodies
//! (uniform FT_PY_RO convention, S-07-23) and `_start` running the module
//! body.
//!
//! 03.02c lowering model:
//! - Module-level names live in MODULE-NAMESPACE GLOBALS (one per name,
//!   G_MOD_BASE.., first-reference order) so `_start` and every function
//!   observe module rebinding. Function-local names live in wasm locals.
//! - Control flow is emitted STRUCTURALLY with explicit br depths
//!   (`ctx.depth` = open structured blocks; loop targets recorded as
//!   absolute depths and subtracted at the br site).
//! - Short-circuit BoolOp and IfExp evaluate lazily into a result local and
//!   produce the operand VALUE (S-01-30/S-01-27).
//! - Calls: proven-static exact-arity module-def calls take the direct
//!   `call` path (S-07-24); everything else builds an args array and goes
//!   through the generic dispatcher (`py_call`, S-07-23) — callable-valued
//!   names, nested defs, lambdas and default-arg calls all ride the
//!   `$function` runtime object.
//! - Closures (S-02-12/13/14): a function that captures or is captured
//!   allocates `$cell` structs per captured name in its PROLOGUE (fresh per
//!   call — recursion reentrancy); a closure's `$function.$cells` array
//!   carries the enclosing frame's cell refs for its free variables; free
//!   cells load from that array at call entry. All closures over one
//!   variable share the cell; late binding falls out.
//!
//! DEVIATION (design doc §6): the prelude is emitted wasm, not Rust-compiled
//! to wasm32 — no wasm32 toolchain exists on this host. See prelude.rs.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::emitter_state::{self as fx, OPS};
use crate::prelude::{self, hc, sget, sset, StrTab};
use crate::prelude_gen::{GS_DONE, GS_START};
use crate::types::*;
use ap_ir::expr::{Arg, BinOp, BoolOp, CmpOp, Expr, UnaryOp};
use ap_ir::item::{Item, Param, Stmt};
use ap_ir::module::Module as IrModule;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, DataSegment, DataSegmentMode, ElementSection,
    Elements, ExportKind, ExportSection, Function, FunctionSection, GlobalSection, GlobalType,
    ImportSection, Instruction, MemorySection, MemoryType, Module, RawSection, RefType,
    TableSection, TableType, ValType,
};

/// Codegen failure. Anything this leaf does not lower is named here —
/// the module is NEVER emitted with a silent miscompile.
#[derive(Debug, PartialEq, Eq)]
pub enum CodegenError {
    /// The IR contains a node this leaf does not lower yet.
    Unsupported(&'static str),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::Unsupported(what) => write!(f, "unsupported IR node: {what}"),
        }
    }
}

impl std::error::Error for CodegenError {}

// ===========================================================================
// Pass 0: the module scan (fail-closed scope check + registration)
// ===========================================================================

/// One lowered user function (module-level `Item::FnDef`, nested
/// `Stmt::LocalDef` or `Expr::Lambda`), in DETERMINISTIC discovery order —
/// the wasm function index is `FN_FIRST_USER + position`.
#[derive(Clone)]
struct UserFn {
    /// Python name (def name or `<lambda>`, S-02-38).
    name: String,
    /// Parameters in source order.
    params: Vec<Param>,
    /// Body statements.
    body: Vec<Stmt>,
    /// Def-time-evaluated default value expressions (defaults only, order).
    defaults: Vec<Expr>,
    /// Enclosing function slot (`usize::MAX` = module scope).
    parent: usize,
    /// Names referenced by this function's body (recursively through nested
    /// defs) that are NOT bound here — resolved against enclosing scopes.
    free: Vec<String>,
    /// `nonlocal`-declared names (cell writes into the enclosing scope).
    nonlocal: Vec<String>,
    /// Captured OWN variables (bound here AND referenced by a descendant) —
    /// cell-backed, slot order = this vec's order. Filled by `resolve`.
    own_cells: Vec<String>,
    /// Free names that resolve to an enclosing CELL (subset of `free`, in
    /// order) — incoming `$cells` array slots. Filled by `resolve`.
    free_cells: Vec<String>,
    /// Leaf 03.04d: generator lowering — true when the body contains
    /// yield/yield-from (03.04d: hidden comp fns pass true for genexps).
    /// The emit pass routes these to FT_GEN_STEP step-fns; call sites build
    /// a parked $generator instead of dispatching.
    is_generator: bool,
}

/// Scan state: fail-closed scope check, function registration, string
/// interning, module-name table.
struct Scan {
    fns: Vec<UserFn>,
    /// Module-level names in first-reference order (global slot = index).
    module_names: Vec<String>,
    /// Names the module actually BINDS (assignment/def/class) — a read of
    /// any other name that names a builtin/exc class resolves to that
    /// class's builtin $type global, not a (never-written) module slot.
    module_binds: Vec<String>,
    /// USER class names (03.03c): a Name-call of one of these routes to
    /// FN_CALL_OF ($type arm -> cls_call __init__ dispatch) instead of the
    /// $function cast (which traps on a class object).
    classes: Vec<String>,
    strings: Vec<Vec<u8>>,
    /// Registration-stack parent slots (nested-def parent links).
    parent_stack: Vec<usize>,
}

impl Scan {
    fn new() -> Self {
        Self {
            fns: Vec::new(),
            module_names: Vec::new(),
            module_binds: Vec::new(),
            classes: Vec::new(),
            strings: Vec::new(),
            parent_stack: Vec::new(),
        }
    }

    fn mod_slot(&mut self, name: &str) {
        if !self.module_names.iter().any(|n| n == name) {
            self.module_names.push(name.to_string());
        }
    }

    fn intern_str(&mut self, s: &str) {
        if !self.strings.iter().any(|b| b.as_slice() == s.as_bytes()) {
            self.strings.push(s.as_bytes().to_vec());
        }
    }
}

fn fn_idx(slot: usize) -> u32 {
    fx::FN_FIRST_USER + slot as u32
}

fn scan_module(module: &IrModule, scan: &mut Scan) -> Result<(), CodegenError> {
    for item in &module.items {
        match item {
            Item::Stmt(stmt) => scan_module_stmt(stmt, scan)?,
            Item::FnDef(fd) => {
                scan.mod_slot(fd.name.as_str());
                if !scan.module_binds.iter().any(|n| n == fd.name.as_str()) {
                    scan.module_binds.push(fd.name.to_string());
                }
                scan_fn_common(
                    fd.name.as_str(),
                    &fd.params,
                    &fd.body.blocks,
                    usize::MAX,
                    fd.is_generator,
                    scan,
                )?;
            }
            Item::ClassDef(cd) => {
                // 03.03: module-level classes — the name is a module
                // global; the body's methods register as plain defs.
                scan.mod_slot(cd.name.as_str());
                if !scan.module_binds.iter().any(|n| n == cd.name.as_str()) {
                    scan.module_binds.push(cd.name.to_string());
                }
                // 03.03c: register as a USER class (Name-call instantiation
                // routes through FN_CALL_OF, see emit_call).
                if !scan.classes.iter().any(|c| c == cd.name.as_str()) {
                    scan.classes.push(cd.name.to_string());
                }
                let mut bound: Vec<String> = Vec::new();
                scan_class_def(cd, &mut bound, scan)?;
                scan_intern_str_in(scan, cd.name.as_str());
            }
        }
    }
    // intern every function/param name AFTER the scan walk (they may not be
    // string literals) so emission can always use segment literals.
    let names: Vec<String> = scan
        .fns
        .iter()
        .flat_map(|f| {
            let mut v = vec![f.name.clone()];
            v.extend(f.params.iter().map(|p| p.name.to_string()));
            v
        })
        .collect();
    for n in names {
        scan.intern_str(&n);
    }
    Ok(())
}

fn scan_fn_common(
    name: &str,
    params: &[Param],
    body: &[Stmt],
    parent: usize,
    is_generator: bool,
    scan: &mut Scan,
) -> Result<(), CodegenError> {
    for p in params {
        if p.default.is_some() {
            // defaults ARE supported (def-time evaluation); the check here
            // is only for parameter KINDS below.
        }
        if !matches!(
            p.kind,
            ap_ir::item::ParamKind::PosOnly | ap_ir::item::ParamKind::PosOrKw
        ) {
            return Err(CodegenError::Unsupported(
                "def parameter kinds beyond positional (post-03.03)",
            ));
        }
    }
    let slot = scan.fns.len();
    scan.fns.push(UserFn {
        name: name.to_string(),
        params: params.to_vec(),
        body: body.to_vec(),
        defaults: params.iter().filter_map(|p| p.default.clone()).collect(),
        parent,
        free: Vec::new(),
        nonlocal: Vec::new(),
        own_cells: Vec::new(),
        free_cells: Vec::new(),
        is_generator,
    });
    // scan the body with this fn as the nesting parent
    let mut refs: Vec<String> = Vec::new();
    let mut bound: Vec<String> = params.iter().map(|p| p.name.to_string()).collect();
    let mut nonlocal: Vec<String> = Vec::new();
    scan.parent_stack.push(slot);
    let r = collect_scope(body, &mut refs, &mut bound, &mut nonlocal, scan);
    scan.parent_stack.pop();
    r?;
    scan.fns[slot].nonlocal = nonlocal.clone();
    for r in refs {
        if bound.contains(&r) || scan.fns[slot].free.contains(&r) {
            continue;
        }
        scan.fns[slot].free.push(r);
    }
    // `nonlocal x` binds x in the ENCLOSING scope (S-02-3/13): force the
    // name into free even though the body assigns it locally.
    for n in &nonlocal {
        if !scan.fns[slot].free.contains(n) {
            scan.fns[slot].free.push(n.clone());
        }
    }
    Ok(())
}

/// Module-level statement scan: registers module names, rejects what this
/// leaf does not lower (fail-closed before ANY emission).
fn scan_module_stmt(stmt: &Stmt, scan: &mut Scan) -> Result<(), CodegenError> {
    let mut refs: Vec<String> = Vec::new();
    let mut bound: Vec<String> = Vec::new();
    let mut nonlocal: Vec<String> = Vec::new();
    scan.parent_stack.push(usize::MAX);
    let r = collect_stmt(stmt, &mut refs, &mut bound, &mut nonlocal, scan);
    scan.parent_stack.pop();
    r?;
    for r in refs {
        scan.mod_slot(&r);
    }
    for b in bound {
        scan.mod_slot(&b);
        if !scan.module_binds.iter().any(|n| n == &b) {
            scan.module_binds.push(b);
        }
    }
    if !nonlocal.is_empty() {
        return Err(CodegenError::Unsupported("nonlocal at module scope (SyntaxError)"));
    }
    Ok(())
}

fn collect_scope(
    body: &[Stmt],
    refs: &mut Vec<String>,
    bound: &mut Vec<String>,
    nonlocal: &mut Vec<String>,
    scan: &mut Scan,
) -> Result<(), CodegenError> {
    for stmt in body {
        collect_stmt(stmt, refs, bound, nonlocal, scan)?;
    }
    Ok(())
}

fn collect_stmt(
    stmt: &Stmt,
    refs: &mut Vec<String>,
    bound: &mut Vec<String>,
    nonlocal: &mut Vec<String>,
    scan: &mut Scan,
) -> Result<(), CodegenError> {
    match stmt {
        Stmt::Line(_) | Stmt::Pass => Ok(()),
        Stmt::ExprStmt(e) => collect_expr(e, refs, bound, nonlocal, scan),
        Stmt::Assign(targets, value) => {
            collect_expr(value, refs, bound, nonlocal, scan)?;
            // 03.03c: attr targets read their object (obj.attr = v) — the
            // attr name rides FN_ATTR_SET as an interned $str at emit time.
            // Name targets are interned too: class-body assigns bind via
            // attr_set(cls, name, v) with an interned name $str.
            for t in targets {
                match t {
                    ap_ir::item::Target::Attribute(obj, name) => {
                        scan.intern_str(name.as_str());
                        collect_expr(obj, refs, bound, nonlocal, scan)?;
                    }
                    ap_ir::item::Target::Name(n) => {
                        scan.intern_str(n.as_str());
                        collect_target(t, bound, scan)?;
                    }
                    _ => collect_target(t, bound, scan)?,
                }
            }
            Ok(())
        }
        Stmt::AugAssign(t, _, v) => {
            collect_target_read(t, refs, bound, nonlocal, scan)?;
            collect_expr(v, refs, bound, nonlocal, scan)
        }
        Stmt::Return(Some(e)) => collect_expr(e, refs, bound, nonlocal, scan),
        Stmt::Return(None) => Ok(()),
        Stmt::If(c, then, els) => {
            collect_expr(c, refs, bound, nonlocal, scan)?;
            collect_scope(then, refs, bound, nonlocal, scan)?;
            collect_scope(els, refs, bound, nonlocal, scan)
        }
        Stmt::While(c, body, els) => {
            collect_expr(c, refs, bound, nonlocal, scan)?;
            collect_scope(body, refs, bound, nonlocal, scan)?;
            collect_scope(els, refs, bound, nonlocal, scan)
        }
        Stmt::Break | Stmt::Continue => Ok(()),
        Stmt::Global(names) => {
            for name in names {
                let s = name.clone().to_string();
                if !refs.contains(&s) {
                    refs.push(s);
                }
            }
            Ok(())
        }
        Stmt::Nonlocal(names) => {
            for name in names {
                let s = name.clone().to_string();
                let dup = refs.contains(&s) || nonlocal.contains(&s);
                if !dup {
                    refs.push(s.clone());
                    nonlocal.push(s);
                } else {
                    if !refs.contains(&s) {
                        refs.push(s.clone());
                    }
                    if !nonlocal.contains(&s) {
                        nonlocal.push(s);
                    }
                }
            }
            Ok(())
        }
        Stmt::LocalDef(fd) => {
            let n = fd.name.to_string();
            if !bound.contains(&n) {
                bound.push(n);
            }
            if !fd.decorators.is_empty() {
                return Err(CodegenError::Unsupported("decorated def (post-03.03)"));
            }
            // leaf 03.04c: generator defs LOWER (is_generator is consumed at
            // emit: the step-fn state machine); async defs stay fail-closed.
            if fd.is_async {
                return Err(CodegenError::Unsupported("async def (post-03.04)"));
            }
            let parent = scan.parent_stack.last().copied().unwrap_or(usize::MAX);
            scan_fn_common(
                fd.name.as_str(),
                &fd.params,
                &fd.body.blocks,
                parent,
                fd.is_generator,
                scan,
            )
        }
        // 03.03: class defs (module-level Item::ClassDef and nested
        // Stmt::LocalClass) register the class NAME and scan the body —
        // the body's methods are plain defs inside the class scope.
        Stmt::LocalClass(cd) => scan_class_def(cd, bound, scan),
        // 03.03: try/except/else/finally + raise (S-02-27..34)
        Stmt::Try(ts) => {
            collect_scope(&ts.body, refs, bound, nonlocal, scan)?;
            for h in &ts.handlers {
                if let Some(ty) = &h.ty {
                    collect_expr(ty, refs, bound, nonlocal, scan)?;
                }
                if let Some(n) = &h.name {
                    let n = n.to_string();
                    if !bound.contains(&n) {
                        bound.push(n);
                    }
                }
                collect_scope(&h.body, refs, bound, nonlocal, scan)?;
            }
            collect_scope(&ts.orelse, refs, bound, nonlocal, scan)?;
            collect_scope(&ts.finalbody, refs, bound, nonlocal, scan)
        }
        Stmt::Raise(e, cause) => {
            if let Some(e) = e {
                collect_expr(e, refs, bound, nonlocal, scan)?;
            }
            if let Some(c) = cause {
                collect_expr(c, refs, bound, nonlocal, scan)?;
            }
            Ok(())
        }
        // leaf 03.04c: For (plain-Name targets) + Match (MA001 slice) scan
        Stmt::For(..) => collect_for(stmt, refs, bound, nonlocal, scan),
        Stmt::Match(..) => collect_match(stmt, refs, bound, nonlocal, scan),
        other => Err(CodegenError::Unsupported(stmt_unsupported(other))),
    }
}

/// leaf 03.04c scan gates for the generator/match slice.
fn collect_for(f: &Stmt, refs: &mut Vec<String>, bound: &mut Vec<String>, nonlocal: &mut Vec<String>, scan: &mut Scan) -> Result<(), CodegenError> {
    let Stmt::For(target, iter, body, els) = f else {
        unreachable!("collect_for on a non-For")
    };
    // target: plain Name only (tuple unpacking targets stay fail-closed)
    if !matches!(target, ap_ir::item::Target::Name(_)) {
        return Err(CodegenError::Unsupported(
            "for target beyond a plain name (03.04c slice)",
        ));
    }
    collect_expr(iter, refs, bound, nonlocal, scan)?;
    collect_target(target, bound, scan)?;
    collect_scope(body, refs, bound, nonlocal, scan)?;
    collect_scope(els, refs, bound, nonlocal, scan)
}

/// leaf 03.04c match scan: subject + per-case gates (MA001 slice — Literal /
/// Capture / Wildcard / Or / Sequence+Starred / Mapping-str-key / Group; Guard
/// exprs collect as reads; Class/Value/Mapping-**rest fail-closed).
fn collect_match(f: &Stmt, refs: &mut Vec<String>, bound: &mut Vec<String>, nonlocal: &mut Vec<String>, scan: &mut Scan) -> Result<(), CodegenError> {
    let Stmt::Match(subject, cases) = f else {
        unreachable!("collect_match on a non-Match")
    };
    collect_expr(subject, refs, bound, nonlocal, scan)?;
    for case in cases {
        collect_pattern(&case.pattern, scan)?;
        if let Some(g) = &case.guard {
            collect_expr(g, refs, bound, nonlocal, scan)?;
        }
        collect_scope(&case.body, refs, bound, nonlocal, scan)?;
        // captures bind in the enclosing scope (PEP 634 store rules)
        let mut names = Vec::new();
        pattern_captures(&case.pattern, &mut names);
        for n in names {
            if !bound.contains(&n) {
                bound.push(n);
            }
        }
    }
    Ok(())
}

fn collect_pattern(p: &ap_ir::item::Pattern, scan: &mut Scan) -> Result<(), CodegenError> {
    use ap_ir::item::Pattern;
    match p {
        Pattern::Literal(e) => collect_pattern_lit(e, scan),
        Pattern::Capture(_) => Ok(()),
        Pattern::Wildcard => Ok(()),
        Pattern::Group(inner) => collect_pattern(inner, scan),
        Pattern::Or(alts) => {
            for a in alts {
                collect_pattern(a, scan)?;
            }
            Ok(())
        }
        Pattern::Sequence(patterns, rest) => {
            for s in patterns {
                collect_pattern(s, scan)?;
            }
            if let Some(r) = rest {
                collect_pattern(r, scan)?;
            }
            Ok(())
        }
        Pattern::Mapping(entries, rest) => {
            if rest.is_some() {
                return Err(CodegenError::Unsupported(
                    "match mapping **rest pattern (03.04c slice)",
                ));
            }
            for (k, v) in entries {
                // str-literal keys only (FN_DGET slice): the key PATTERN must
                // be a Literal; its expr gets the constant gate
                let Pattern::Literal(ke) = k else {
                    return Err(CodegenError::Unsupported(
                        "match mapping key beyond a literal (03.04c)",
                    ));
                };
                collect_pattern_lit(ke, scan)?;
                collect_pattern(v, scan)?;
            }
            Ok(())
        }
        Pattern::Value(_) => Err(CodegenError::Unsupported(
            "match Value pattern (03.04c slice)",
        )),
        Pattern::Class(..) => Err(CodegenError::Unsupported(
            "match Class pattern (03.04c slice)",
        )),
    }
}

/// Literal-pattern expressions: the constants the FN_EQ compare slice covers
/// (int/str/bool/None literals); anything else fails closed.
fn collect_pattern_lit(e: &Expr, scan: &mut Scan) -> Result<(), CodegenError> {
    match e {
        Expr::LitInt(_) | Expr::LitBigInt(_) | Expr::LitBool(_) | Expr::LitNone => Ok(()),
        Expr::LitStr(s) => {
            scan.intern_str(s);
            Ok(())
        }
        _ => Err(CodegenError::Unsupported(
            "match literal pattern beyond the constant slice (03.04c)",
        )),
    }
}

/// Capture names under a pattern, in order (dedup).
fn pattern_captures(p: &ap_ir::item::Pattern, out: &mut Vec<String>) {
    use ap_ir::item::Pattern;
    match p {
        Pattern::Capture(n) => {
            let n = n.to_string();
            if !out.contains(&n) {
                out.push(n);
            }
        }
        Pattern::Group(inner) => pattern_captures(inner, out),
        Pattern::Or(alts) => {
            for a in alts {
                pattern_captures(a, out);
            }
        }
        Pattern::Sequence(patterns, rest) => {
            for s in patterns {
                pattern_captures(s, out);
            }
            if let Some(r) = rest {
                pattern_captures(r, out);
            }
        }
        Pattern::Mapping(entries, _) => {
            for (_, v) in entries {
                pattern_captures(v, out);
            }
        }
        Pattern::Literal(_) | Pattern::Wildcard | Pattern::Value(_) | Pattern::Class(..) => {}
    }
}

/// Scan a class definition (leaf 03.03 slice): bind the class name, scan
/// the body. Decorators/metaclass/multi-base are the caller's fail-closed
/// rejects at emit time (S-03-1/2 recorded there).
fn scan_class_def(
    cd: &ap_ir::item::ClassDef,
    bound: &mut Vec<String>,
    scan: &mut Scan,
) -> Result<(), CodegenError> {
    let n = cd.name.to_string();
    // intern the class name (emit builds the class via FN_CLS_NEW's name $str —
    // nested class defs never ran the Item::ClassDef intern)
    scan.intern_str(&n);
    if !bound.contains(&n) {
        bound.push(n);
    }
    let mut refs: Vec<String> = Vec::new();
    let mut b2: Vec<String> = Vec::new();
    let mut nl: Vec<String> = Vec::new();
    scan.parent_stack.push(usize::MAX);
    let r = collect_scope(&cd.body, &mut refs, &mut b2, &mut nl, scan);
    scan.parent_stack.pop();
    r?;
    for r in refs {
        scan.mod_slot(&r);
    }
    for b in b2 {
        scan.mod_slot(&b);
    }
    Ok(())
}

/// Assignment target: binds (name) or reads (subscript/attribute obj+key).
fn collect_target(
    t: &ap_ir::item::Target,
    bound: &mut Vec<String>,
    _scan: &mut Scan,
) -> Result<(), CodegenError> {
    match t {
        ap_ir::item::Target::Name(n) => {
            let n = n.to_string();
            if !bound.contains(&n) {
                bound.push(n);
            }
            Ok(())
        }
        _ => Err(CodegenError::Unsupported(
            "unpacking/subscript/attr assign target (post-03.03)",
        )),
    }
}

fn collect_target_read(
    t: &ap_ir::item::Target,
    refs: &mut Vec<String>,
    bound: &mut Vec<String>,
    nonlocal: &mut Vec<String>,
    scan: &mut Scan,
) -> Result<(), CodegenError> {
    match t {
        ap_ir::item::Target::Name(_) => collect_target(t, bound, scan), // read+write: bound
        _ => {
            let mut b2 = Vec::new();
            collect_target(t, &mut b2, scan)?;
            // subscript/attr target reads its object — walk via exprs
            match t {
                ap_ir::item::Target::Subscript(o, k) => {
                    collect_expr(o, refs, bound, nonlocal, scan)?;
                    collect_expr(k, refs, bound, nonlocal, scan)
                }
                ap_ir::item::Target::Attribute(o, _) => {
                    collect_expr(o, refs, bound, nonlocal, scan)
                }
                _ => unreachable!(),
            }
        }
    }
}

fn collect_expr(
    e: &Expr,
    refs: &mut Vec<String>,
    bound: &mut Vec<String>,
    nonlocal: &mut Vec<String>,
    scan: &mut Scan,
) -> Result<(), CodegenError> {
    match e {
        Expr::LitInt(_)
        | Expr::LitBigInt(_)
        | Expr::LitFloat(_)
        | Expr::LitBool(_)
        | Expr::LitNone
        | Expr::LitEllipsis => Ok(()),
        Expr::LitStr(s) => {
            scan.intern_str(s);
            Ok(())
        }
        Expr::LitBytes(_) => Err(CodegenError::Unsupported("bytes literal (03.03 surface)")),
        Expr::Name(n) => {
            let n = n.to_string();
            // 03.03c fail-closed: names this slice cannot resolve at emit
            // (emit_name_load would panic) reject at scan instead.
            if matches!(n.as_str(), "super" | "NotImplemented") {
                return Err(CodegenError::Unsupported(
                    "name beyond the 03.03 slice (super/NotImplemented)",
                ));
            }
            if !refs.contains(&n) {
                refs.push(n);
            }
            Ok(())
        }
        Expr::BinOp(_, a, b) => {
            collect_expr(a, refs, bound, nonlocal, scan)?;
            collect_expr(b, refs, bound, nonlocal, scan)
        }
        Expr::UnaryOp(_, a) => collect_expr(a, refs, bound, nonlocal, scan),
        Expr::BoolOp(_, ops) => {
            for o in ops {
                collect_expr(o, refs, bound, nonlocal, scan)?;
            }
            Ok(())
        }
        Expr::Compare(_, operands) => {
            for o in operands {
                collect_expr(o, refs, bound, nonlocal, scan)?;
            }
            Ok(())
        }
        Expr::IfExp(c, t, e) => {
            collect_expr(c, refs, bound, nonlocal, scan)?;
            collect_expr(t, refs, bound, nonlocal, scan)?;
            collect_expr(e, refs, bound, nonlocal, scan)
        }
        Expr::Attribute(obj, name) => {
            // the attribute name rides FN_ATTR_GET as an interned $str
            scan.intern_str(name.as_str());
            collect_expr(obj, refs, bound, nonlocal, scan)
        }
        Expr::Call(f, args) => {
            // 03.03d: type(x) as a CALLEE (1 positional arg) is lowered — the
            // raw type-field read. 3-arg metaclass form stays rejected.
            if let Expr::Name(n) = f.as_ref() {
                if n.as_str() == "type" {
                    let npos = args.iter().filter(|a| matches!(a, Arg::Pos(_))).count();
                    if args.iter().all(|a| matches!(a, Arg::Pos(_))) && npos == 1 {
                        if let Arg::Pos(x) = &args[0] {
                            return collect_expr(x, refs, bound, nonlocal, scan);
                        }
                    }
                    return Err(CodegenError::Unsupported(
                        "type() form beyond the 03.03d slice (n-ary/kwarg)",
                    ));
                }
            }
            // 03.03d: super(...) as a CALLEE is lowered (S-03-17 slice); the
            // bare name outside a call stays rejected. Arity: 0 (bare, raises
            // RuntimeError) or 2 (two-arg build), positional only.
            if let Expr::Name(n) = f.as_ref() {
                if n.as_str() == "super" {
                    let npos = args
                        .iter()
                        .filter(|a| matches!(a, Arg::Pos(_)))
                        .count();
                    if !args.iter().all(|a| matches!(a, Arg::Pos(_)))
                        || (npos != 0 && npos != 2)
                    {
                        return Err(CodegenError::Unsupported(
                            "super() form beyond the 03.03d slice (kwargs/n-ary)",
                        ));
                    }
                    for a in args {
                        if let Arg::Pos(x) = a {
                            collect_expr(x, refs, bound, nonlocal, scan)?;
                        }
                    }
                    return Ok(());
                }
            }
            collect_expr(f, refs, bound, nonlocal, scan)?;
            for a in args {
                match a {
                    Arg::Pos(x) => collect_expr(x, refs, bound, nonlocal, scan)?,
                    _ => {
                        return Err(CodegenError::Unsupported(
                            "call arg kinds beyond positional (03.03)",
                        ))
                    }
                }
            }
            Ok(())
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                collect_expr(i, refs, bound, nonlocal, scan)?;
            }
            Ok(())
        }
        Expr::Dict(items) => {
            for it in items {
                collect_expr(&it.key, refs, bound, nonlocal, scan)?;
                collect_expr(&it.value, refs, bound, nonlocal, scan)?;
            }
            Ok(())
        }
        Expr::Lambda(sig, body) => {
            for p in &sig.params {
                if p.default.is_some() {
                    return Err(CodegenError::Unsupported("lambda default (post-03.03)"));
                }
                if !matches!(
                    p.kind,
                    ap_ir::item::ParamKind::PosOnly | ap_ir::item::ParamKind::PosOrKw
                ) {
                    return Err(CodegenError::Unsupported("lambda param kind (post-03.03)"));
                }
            }
            let body_stmts = vec![Stmt::Return(Some(body.as_ref().clone()))];
            let parent = scan.parent_stack.last().copied().unwrap_or(usize::MAX);
            scan_fn_common("<lambda>", &sig.params, &body_stmts, parent, false, scan)
        }
        // leaf 03.04c: comprehension scopes. The clause-0 iterable evaluates
        // in the ENCLOSING scope (S-02-5) — collect its reads HERE; the
        // target/element/ifs collect as the hidden <comp> function's body
        // via collect_comp_clause (which registers the hidden fn).
        Expr::ListComp(elt, clauses) | Expr::Generator(elt, clauses) => {
            collect_comp_clauses(elt, clauses, refs, bound, nonlocal, scan)
        }
        Expr::DictComp(key, val, clauses) => {
            let mut dummy = Expr::Tuple(vec![(*key.clone()), (*val.clone())]);
            let r = collect_comp_clauses(&dummy, clauses, refs, bound, nonlocal, scan);
            drop(dummy);
            r
        }
        Expr::SetComp(..) => Err(CodegenError::Unsupported(
            "set comprehension (fail-closed, 03.04c)",
        )),
        // `yield` / `yield from` OUTSIDE a generator def cannot be scanned
        // here (the generator-ness of the ENCLOSING fn decides); the
        // fn-scan (scan_fn_common) accepts them via this same arm when its
        // `in_generator` gate (below) allows. This arm is the SHARED gate:
        // collect_expr doesn't know the context, so allow + collect; the
        // module-scope reject happens via scan_module's post-check.
        Expr::Yield(v) => {
            if let Some(v) = v {
                collect_expr(v, refs, bound, nonlocal, scan)?;
            }
            Ok(())
        }
        Expr::YieldFrom(v) => {
            collect_expr(v, refs, bound, nonlocal, scan)?;
            // iter(x) + attr reads the delegation needs (interned "value")
            scan.intern_str("value");
            Ok(())
        }
        Expr::Named(t, v) => {
            // walrus binds the NAME in the enclosing scope (S-02-2)
            let Expr::Name(n) = t.as_ref() else {
                return Err(CodegenError::Unsupported("walrus non-name target"));
            };
            let n = n.to_string();
            scan.intern_str(&n);
            collect_expr(v, refs, bound, nonlocal, scan)?;
            if !bound.contains(&n) {
                bound.push(n);
            }
            Ok(())
        }
        Expr::Subscript(obj, key) => {
            collect_expr(obj, refs, bound, nonlocal, scan)?;
            collect_expr(key, refs, bound, nonlocal, scan)
        }
        Expr::Starred(x) => collect_expr(x, refs, bound, nonlocal, scan),
        _ => Err(CodegenError::Unsupported("Expr (03.02c slice)")),
    }
}

/// The comprehension hidden-function walk (03.04c): registers a <comp>
/// UserFn whose body is the nested for/if structure, then collects the
/// element (or key/value) reads INSIDE that fn scope. Clause 0's iterable
/// was already collected by the CALLER (enclosing scope).
fn collect_comp_clauses(
    elt: &Expr,
    clauses: &[ap_ir::expr::CompClause],
    refs: &mut Vec<String>,
    bound: &mut Vec<String>,
    nonlocal: &mut Vec<String>,
    scan: &mut Scan,
) -> Result<(), CodegenError> {
    if clauses.is_empty() {
        return Err(CodegenError::Unsupported("comprehension without clauses"));
    }
    // clause 0 iterable rides the caller's collection; nested clauses'
    // iterables collect INSIDE the hidden fn body.
    let mut body: Vec<Stmt> = Vec::new();
    // innermost-first construction: element append
    let inner = vec![Stmt::ExprStmt(elt.clone())];
    let mut cur = inner;
    for clause in clauses.iter().rev() {
        // iterables of clauses >= 1 collect inside the hidden fn
        let mut crefs: Vec<String> = Vec::new();
        collect_expr(&clause.iter, &mut crefs, bound, nonlocal, scan)?;
        let mut ifs = Vec::new();
        for i in &clause.ifs {
            let mut irefs = Vec::new();
            collect_expr(i, &mut irefs, bound, nonlocal, scan)?;
            ifs.push(i);
            // the If lowering reads these through `refs` — re-add
        }
        body.clear();
        // build: For(target, iter, [if-chain + cur], [])
        let mut stmt_body = cur;
        // wrap ifs outermost-innermost: For body = if1{if2{...cur}}
        for i in ifs.iter().rev() {
            stmt_body = vec![Stmt::If((**i).clone(), stmt_body, Vec::new())];
        }
        let target = comp_target(&clause.target)?;
        cur = vec![Stmt::For(
            target,
            clause.iter.clone(),
            stmt_body,
            Vec::new(),
        )];
        body = cur.clone();
        for r in crefs {
            if !refs.contains(&r) {
                refs.push(r);
            }
        }
        let _ = &mut ifs;
        let _ = &clause.iter;
    }
    let parent = scan.parent_stack.last().copied().unwrap_or(usize::MAX);
    scan_fn_common("<listcomp>", &[], &body, parent, false, scan)
}

/// comprehension clause target: Name or 1-level Tuple of Names (the corpus
/// slice: `for y in x` / nested `for y in [(1,2)] for y in x` unpacks via
/// the Tuple arm only when all-Name; deeper shapes fail closed).
fn comp_target(t: &Expr) -> Result<ap_ir::item::Target, CodegenError> {
    match t {
        Expr::Name(n) => Ok(ap_ir::item::Target::Name(n.clone())),
        Expr::Tuple(items) => {
            let mut out = Vec::new();
            for i in items {
                let Expr::Name(n) = i else {
                    return Err(CodegenError::Unsupported(
                        "comp tuple target beyond all-Name (03.04c)",
                    ));
                };
                out.push(ap_ir::item::Target::Name(n.clone()));
            }
            Ok(ap_ir::item::Target::Tuple(out))
        }
        _ => Err(CodegenError::Unsupported(
            "comp target beyond Name/Tuple-of-Names (03.04c)",
        )),
    }
}

fn stmt_unsupported(s: &Stmt) -> &'static str {
    match s {
        Stmt::For(..) => "Stmt::For (leaf 03.04: iterators)",
        Stmt::Raise(..) => "Stmt::Raise form beyond the slice (post-03.03)",
        Stmt::Match(..) => "Stmt::Match (leaf 03.04)",
        Stmt::With(..) => "Stmt::With (03.03)",
        Stmt::Assert(..) => "Stmt::Assert (03.03)",
        Stmt::AnnAssign(..) => "Stmt::AnnAssign (post-03.03)",
        Stmt::Import(_) | Stmt::ImportFrom(..) => "import (Plan 03 stdlib surface)",
        Stmt::Del(_) => "Stmt::Del (post-03.03)",
        _ => "Stmt (03.02c scope)",
    }
}

// ===========================================================================
// Name resolution (cells)
// ===========================================================================

/// Where a name resolves inside one function scope.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Res {
    /// This function's own cell slot.
    OwnCell(u32),
    /// Local (param or plain local) — wasm local index.
    Local(u32),
    /// Incoming free-cell slot (enclosing frame's cell).
    FreeCell(u32),
    /// Module namespace global.
    Global,
}

/// Compute `own_cells`/`free_cells` for every registered function.
/// Order guarantee: parents register before children (discovery order), so
/// a child's resolution can consult its parent's completed tables.
fn resolve_cells(scan: &mut Scan) {
    // bound names per fn (params + body assigns + nested def names)
    let bounds: Vec<Vec<String>> = scan
        .fns
        .iter()
        .map(|f| {
            let mut b: Vec<String> = f.params.iter().map(|p| p.name.to_string()).collect();
            for n in collect_local_binds(&f.body) {
                if !b.contains(&n) {
                    b.push(n);
                }
            }
            b
        })
        .collect();
    // own cells: bound here AND in some strict descendant's free set
    let is_descendant = |scan: &Scan, anc: usize, node: usize| -> bool {
        let mut p = scan.fns[node].parent;
        while p != usize::MAX {
            if p == anc {
                return true;
            }
            p = scan.fns[p].parent;
        }
        false
    };
    for i in 0..scan.fns.len() {
        let mut own: Vec<String> = Vec::new();
        for name in &bounds[i] {
            let captured = (0..scan.fns.len())
                .filter(|&j| j != i && is_descendant(scan, i, j))
                .any(|j| scan.fns[j].free.contains(name) || scan.fns[j].nonlocal.contains(name));
            if captured && !own.contains(name) {
                own.push(name.clone());
            }
        }
        scan.fns[i].own_cells = own;
    }
    // free_cells per fn: free names whose PARENT-scope resolution is a cell
    for i in 0..scan.fns.len() {
        let parent = scan.fns[i].parent;
        let mut fc: Vec<String> = Vec::new();
        for name in &scan.fns[i].free {
            let cell_visible = if parent == usize::MAX {
                false // module scope has no cells
            } else {
                scan.fns[parent].own_cells.contains(name)
                    || scan.fns[parent].free_cells.contains(name)
            };
            if cell_visible && !fc.contains(name) {
                fc.push(name.clone());
            }
        }
        scan.fns[i].free_cells = fc;
    }
}

/// The resolution map for function `slot`'s body (cells pass done).
fn resolution_map(scan: &Scan, slot: usize) -> std::collections::HashMap<String, Res> {
    let mut map = std::collections::HashMap::new();
    let f = &scan.fns[slot];
    for (i, n) in f.own_cells.iter().enumerate() {
        map.insert(n.clone(), Res::OwnCell(i as u32));
    }
    // Python params live at locals 3..3+n (the FT_PY_RO convention owns
    // 0 = $function, 1 = args array, 2 = kwargs); plain locals follow.
    let mut plain = 0u32;
    let py_param_base = 3u32;
    for (i, p) in f.params.iter().enumerate() {
        map.entry(p.name.to_string())
            .or_insert(Res::Local(py_param_base + i as u32));
    }
    for n in collect_local_binds(&f.body) {
        if f.params.iter().any(|p| p.name == n) {
            continue;
        }
        if f.own_cells.contains(&n) {
            continue; // cell-backed plain local: no separate local slot
        }
        if f.free.contains(&n) || f.free_cells.contains(&n) {
            continue; // free variable: enclosing cell, not a local (S-02-12)
        }
        map.entry(n).or_insert(Res::Local(
            py_param_base + f.params.len() as u32 + plain,
        ));
        plain += 1;
    }
    for (i, n) in f.free_cells.iter().enumerate() {
        map.insert(n.clone(), Res::FreeCell(i as u32));
    }
    for n in &f.free {
        map.entry(n.clone()).or_insert(Res::Global);
    }
    map
}

/// Names ASSIGN-bound in a function body (nested defs bind their NAME here;
/// their internal scopes do not leak).
fn collect_local_binds(body: &[Stmt]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    fn walk(stmts: &[Stmt], out: &mut Vec<String>) {
        for s in stmts {
            match s {
                Stmt::Assign(targets, _) => {
                    for t in targets {
                        if let ap_ir::item::Target::Name(n) = t {
                            let n = n.to_string();
                            if !out.contains(&n) {
                                out.push(n);
                            }
                        }
                    }
                }
                Stmt::AugAssign(t, _, _) => {
                    if let ap_ir::item::Target::Name(n) = t {
                        let n = n.to_string();
                        if !out.contains(&n) {
                            out.push(n);
                        }
                    }
                }
                Stmt::LocalDef(fd) => {
                    let n = fd.name.to_string();
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
                Stmt::LocalClass(cd) => {
                    let n = cd.name.to_string();
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
                Stmt::Try(ts) => {
                    for h in &ts.handlers {
                        if let Some(n) = &h.name {
                            let n = n.to_string();
                            if !out.contains(&n) {
                                out.push(n);
                            }
                        }
                        walk(&h.body, out);
                    }
                    walk(&ts.body, out);
                    walk(&ts.orelse, out);
                }
                Stmt::If(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Stmt::While(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                // leaf 03.04d: For targets bind in the enclosing fn scope
                // (generator bodies rely on this for slot tables).
                Stmt::For(target, _, a, b) => {
                    if let ap_ir::item::Target::Name(n) = target {
                        let n = n.to_string();
                        if !out.contains(&n) {
                            out.push(n);
                        }
                    }
                    walk(a, out);
                    walk(b, out);
                }
                _ => {}
            }
        }
    }
    walk(body, &mut out);
    out
}

// ===========================================================================
// lower_module
// ===========================================================================

/// Lower a module to wasm binary bytes.
///
/// Section order is normative (S-07-43): types, imports, functions, tables,
/// element, globals, memory, exports, code, data, custom(`aprt-version`).
pub fn lower_module(module: &IrModule) -> Result<Vec<u8>, CodegenError> {
    lower_module_ex(module, None)
}

/// lower_module with the source FILENAME (leaf 03.03 scope 6/7: traceback
/// frames + the host-boundary render print `File "<name>", line N` — the
/// corpus gate passes the case path so the uncaught-traceback stderr is
/// byte-identical with CPython's).
pub fn lower_module_ex(
    module: &IrModule,
    source_name: Option<&str>,
) -> Result<Vec<u8>, CodegenError> {
    let _source_name_guard = source_name;
    // ---- pass 0: scan (fail-closed) + cell resolution ----
    let mut scan = Scan::new();
    scan_module(module, &mut scan)?;
    resolve_cells(&mut scan);
    let res_maps: Vec<std::collections::HashMap<String, Res>> =
        (0..scan.fns.len()).map(|i| resolution_map(&scan, i)).collect();

    // ---- data-segment image: [name blob][module literals][prelude] ----
    // name blob = TYPE_ORDER names + the EXC_TREE names (exception class
    // name strings; leaf 03.03)
    let name_blob: Vec<u8> = TYPE_ORDER
        .iter()
        .chain(EXC_TREE.iter().map(|(n, _)| n))
        .flat_map(|n| n.as_bytes())
        .copied()
        .collect();
    let strings_snapshot = scan.strings.clone();
    let mut tab = StrTab::new(name_blob.clone(), &strings_snapshot);
    let _prelude_base = tab.prelude_base();

    let mut m = Module::new();

    // ---- type section ----
    // ---- import section (S-07-41) ----
    let mut imports = ImportSection::new();
    imports.import(
        "wasi_snapshot_preview1",
        "fd_write",
        wasm_encoder::EntityType::Function(FT_FD_WRITE),
    );
    imports.import(
        "aprt",
        "abort_msg",
        wasm_encoder::EntityType::Function(FT_ABORT_MSG),
    );
    // 03.03: proc_exit import (index 2 — the fn-index consts in
    // emitter_state.rs assume exactly these 3 imports)
    imports.import(
        "wasi_snapshot_preview1",
        "proc_exit",
        wasm_encoder::EntityType::Function(FT_PROC_EXIT),
    );

    // ---- function section: rt_init(2), prelude, user fns, _start ----
    let mut funcs = FunctionSection::new();
    funcs.function(FT_VOID); // 2 rt_init
    let mut code = CodeSection::new();
    {
        let mut f = Function::new([(1, ValType::I32)]);
        f.instruction(&Instruction::GlobalGet(G_RT_DONE));
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        emit_rt_init_ordered(&mut f);
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    prelude::emit(&mut funcs, &mut code, &mut tab);
    // leaf 03.03: classes + exceptions prelude (frames, dict ops, MRO walk).
    // FN_FIRST_USER (emitter_state) is the tail of this block. Frame
    // globals sit AFTER the module namespace — indices computed at scan.
    let g = crate::prelude_classes::FrameGlobals {
        frame_head: G_MOD_BASE + scan.module_names.len() as u32,
        frame_depth: G_MOD_BASE + scan.module_names.len() as u32 + 1,
        cur_line: G_MOD_BASE + scan.module_names.len() as u32 + 2,
        source_file: G_MOD_BASE + scan.module_names.len() as u32 + 3,
        active_exc: G_MOD_BASE + scan.module_names.len() as u32 + 4,
        pending_exc: G_MOD_BASE + scan.module_names.len() as u32 + 5,
    };
    crate::prelude_classes::emit_all(&mut funcs, &mut code, &mut tab, &g);
    // leaf 03.04: generators + iterator drivers (dense append after 61;
    // FN_FIRST_USER=69 is the tail of this block — never renumber).
    crate::prelude_gen::emit_gen_prelude(&mut funcs, &mut code, &mut tab);
    // leaf 03.03d: super() build (FN_SUPER_NEW = 69) — dense append after the
    // 03.04 drivers; FN_FIRST_USER moved 69 -> 70 (emitter_state).
    crate::prelude_classes::emit_super(&mut funcs, &mut code, &g, &mut tab);
    // leaf 03.04c: drain drivers (FN_LIST_DRAIN=71 / FN_SUM_DRAIN=72) —
    // AFTER emit_super so the 69/70 constants stay true (emit order IS
    // the index assignment).
    crate::prelude_gen::emit_gen_drains(&mut funcs, &mut code);

    // ---- 03.03b: the top-level EH wrap is ON BY DEFAULT. The parked
    // "operators remaining / values remaining on stack" rejection was NOT a
    // try_table shape problem — the wrap-off path built the module-frame
    // name $str UNCONDITIONALLY but gated only the frame_push CALL on the
    // flag, leaving the $str dangling on _start's operand stack. With the
    // name build moved inside the gate, both paths validate (diag_run033
    // 'pass'/'x=1'/'print(1)' probes, wrap on AND off).
    let eh_wrap = true;

    // ---- user functions ----
    let fns_meta: Vec<UserFn> = scan.fns.clone();
    let n_user = fns_meta.len();
    for (slot, uf) in fns_meta.iter().enumerate() {
        if uf.is_generator {
            funcs.function(FT_GEN_STEP);
        } else {
            funcs.function(FT_PY_RO);
        }
        let f = emit_user_fn(uf, slot, &scan, &res_maps[slot], &tab, &g, eh_wrap);
        code.function(&f);
    }

    // ---- table (funcref; entries appended via the element section) ----
    // call_indirect dispatch table: plain `funcref` (the type check is the
    // FT_PY_RO signature at the call site; the $code.$func_idx entries are
    // plain function indices — a concrete-GC-type table would reject
    // funcref elements). $function objects are GC structs, a DIFFERENT
    // representation from the table's funcref entries.
    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType {
            nullable: true,
            heap_type: wasm_encoder::HeapType::Abstract {
                shared: false,
                ty: wasm_encoder::AbstractHeapType::Func,
            },
        },
        minimum: ((n_user as u64) + 1).max(1), // +1 spare: prelude drivers call_indirect
        maximum: None,
        table64: false,
        shared: false,
    });

    // ---- memory ----
    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 2,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });

    // ---- globals: singletons + rt_done + types + complex + module names ----
    let mut globals = GlobalSection::new();
    let add_global = |g: &mut GlobalSection, ty: ValType, init_heap: wasm_encoder::HeapType| {
        g.global(
            GlobalType {
                val_type: ty,
                mutable: true,
                shared: false,
            },
            &ConstExpr::extended([Instruction::RefNull(init_heap)]),
        );
    };
    for _ in 0..4 {
        add_global(
            &mut globals,
            pyobj_ref(),
            wasm_encoder::HeapType::Concrete(T_PYOBJ),
        );
    }
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::extended([Instruction::I32Const(0)]),
    ); // rt_done
    // builtin types + the exception tree (leaf 03.03: rt_init builds the
    // EXC_TREE $type objects into these slots)
    for _ in 0..TYPE_ORDER.len() + EXC_TREE.len() {
        add_global(
            &mut globals,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_TYPE),
            }),
            wasm_encoder::HeapType::Concrete(T_TYPE),
        );
    }
    add_global(
        &mut globals,
        pyobj_ref(),
        wasm_encoder::HeapType::Concrete(T_PYOBJ),
    ); // complex slot (G_COMPLEX)
       // module namespace: one mutable pyobj global per name (null = unbound)
    for _ in 0..scan.module_names.len() {
        add_global(
            &mut globals,
            pyobj_ref(),
            wasm_encoder::HeapType::Concrete(T_PYOBJ),
        );
    }
    // leaf 03.03: frame-stack head ($tb_frame ref), frame depth (i32),
    // current line (i32), source filename ($str), active exception (oref).
    globals.global(
        GlobalType {
            val_type: ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_TB_FRAME),
            }),
            mutable: true,
            shared: false,
        },
        &ConstExpr::extended([Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_TB_FRAME,
        ))]),
    ); // G_FRAME_HEAD
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::extended([Instruction::I32Const(0)]),
    ); // G_FRAME_DEPTH
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::extended([Instruction::I32Const(0)]),
    ); // G_CUR_LINE
    globals.global(
        GlobalType {
            val_type: ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_STR),
            }),
            mutable: true,
            shared: false,
        },
        &ConstExpr::extended([Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_STR,
        ))]),
    ); // G_SOURCE_FILE
    globals.global(
        GlobalType {
            val_type: pyobj_ref(),
            mutable: true,
            shared: false,
        },
        &ConstExpr::extended([Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_PYOBJ,
        ))]),
    ); // G_ACTIVE_EXC
    globals.global(
        GlobalType {
            val_type: pyobj_ref(),
            mutable: true,
            shared: false,
        },
        &ConstExpr::extended([Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_PYOBJ,
        ))]),
    ); // G_PENDING_EXC (module-boundary escape hatch for tb_render)

    // NOTE: the data-segment blob is snapshotted AFTER _start emission —
    // the EH-wrap prologue pushes the source filename + "<module>" into the
    // string table (03.03b fix: the early snapshot truncated the segment and
    // array.new_data trapped out-of-bounds at the module-frame push).

        // ---- _start: rt_init, module-def function objects, module body ----
        {
            // (eh_wrap hoisted to lower_module_ex scope)
            let mut locals: Vec<(u32, ValType)> = Vec::new();
        // filler to index 40, then the shared temp block (mirrors 03.02b)
        locals.push((40, pyobj_ref()));
        locals.push((1, ValType::I32)); // 40: print-buffer running end
        locals.push((1, ValType::I64)); // 41
        locals.push((2, pyobj_ref())); // 42..43
        locals.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR),
            }),
        )); // 44: mut $obj_arr stash (param-name arrays)
        locals.push((1, pyobj_ref())); // 45
        locals.push((1, ValType::I64)); // 46: spare
        locals.push((1, ValType::I32)); // 47: compare spill
        locals.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR_RO),
            }),
        )); // 48: array.new_fixed stash
        locals.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_FUNCTION),
            }),
        )); // 49: $function stash
        locals.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR),
            }),
        )); // 50: second mut $obj_arr stash (param_names)
        locals.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_CELL_ARR),
            }),
        )); // 51: $cell_arr stash (free-cell load; unused in _start)
        locals.push((1, pyobj_ref())); // 52: iterator stash (03.04c For/gen)
        // 03.04c range() scratch: 53/54/55 = i64 (start/stop/step),
        // 56/59 = i32 (count, i), 57 = mut $obj_arr snapshot.
        locals.push((3, ValType::I64)); // 53..55
        locals.push((1, ValType::I32)); // 56 = range count
        locals.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR),
            }),
        )); // 57: range snapshot
        locals.push((1, ValType::I32)); // 58: spare i32 (padding to 59)
        locals.push((1, ValType::I32)); // 59: range loop i
        funcs.function(FT_VOID); // _start entry
        let mut f = Function::new(locals);
        f.instruction(&Instruction::Call(fx::FN_RT_INIT));
        // TEMP_LOCAL(40) starts at PRINT_BUF (fresh-frame zero would make
        // the first flush compute a negative length — the 03.02a regression)
        f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
        f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
        let mut buf_ptr = fx::PRINT_BUF;
        // def-time $function objects for MODULE-level defs, in registration
        // order (source order). Nested defs bind inside their parent body.
        let module_scope = Scope {
            kind: ScopeKind::Module,
            res: std::collections::HashMap::new(),
            cell_base: 0,
            n_own: 0,
            params: 0,
        };
        for uf in &fns_meta {
            if uf.parent != usize::MAX || uf.name == "<lambda>" {
                // lambdas build their object at the use site (no def-time
                // name binding); nested defs bind inside their parent body
                continue;
            }
            let mut ctx = EmitCtx {
                scan: &scan,
                res_maps: &res_maps,
                tab: &tab,
                strs: &scan.strings,
                buf_ptr: &mut buf_ptr,
                scope: &module_scope,
                depth: 0,
                br_continue: None,
                br_break: None,
                cls_obj: None,
                fg: g,
                eh_wrap_top: eh_wrap,
            };
            let slot = scan
                .fns
                .iter()
                .position(|c| std::ptr::eq(c as *const UserFn, uf as *const UserFn))
                .unwrap_or_else(|| {
                    fns_meta
                        .iter()
                        .position(|c| c.name == uf.name && c.parent == usize::MAX)
                        .expect("module def registered")
                });
            emit_function_object(&mut f, slot, &mut ctx);
            f.instruction(&Instruction::GlobalSet(mod_global(
                &uf.name,
                &scan.module_names,
            )));
        }
        // ---- 03.03 traceback prologue + top-level EH wrap (ON by default) ----
        //   Block $catch (result (ref null $exception))   <- handler label 1
        //     try_table (empty) [Catch tag -> label 0 = $catch]
        //       <prologue + module body>
        //     end
        //     RefNull $exception; br 0   -> $catch end (success, null payload)
        //   end ($catch: payload on the stack — real exc on throw, null on
        //   success)
        //   null -> continue to _start's End; non-null -> tb_render + rc 1.
        // Shape proven by probe_payload_tag (eh_probe.rs): the Catch label
        // resolves in the scope ENCLOSING the try_table, and after the
        // try_table's End the label-1 target is the enclosing Block.
        if eh_wrap {
            f.instruction(&Instruction::Block(BlockType::Result(
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_EXCEPTION),
                }),
            ))); // $catch
        }
        {
            let catches = [wasm_encoder::Catch::One {
                tag: TAG_PYEXC,
                label: 0, // $catch (enclosing block)
            }];
            if eh_wrap {
                f.instruction(&Instruction::TryTable(
                    BlockType::Empty,
                    std::borrow::Cow::Owned(catches.to_vec()),
                ));
            }
        }
        if let Some(name) = source_name {
            // G_SOURCE_FILE = the filename $str (prelude string blob)
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("str").unwrap_or(0),
            ));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(name.len() as i32));
            let (s_off, s_len) = tab.push(name);
            f.instruction(&Instruction::I32Const(s_off as i32));
            f.instruction(&Instruction::I32Const(s_len as i32));
            f.instruction(&Instruction::ArrayNewData {
                array_type_index: T_BYTE_ARR,
                array_data_index: 1,
            });
            f.instruction(&Instruction::StructNew(T_STR));
            f.instruction(&Instruction::GlobalSet(g.source_file));
        }
        // module frame push ("<module>") — the whole name-$str build is
        // inside the gate: an unconditional build + gated CALL left the
        // $str dangling on the operand stack (the parked "values remaining
        // on stack" validate rejection root).
        if eh_wrap {
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("str").unwrap_or(0),
            ));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(8));
            let (m_off, m_len) = tab.push("<module>");
            f.instruction(&Instruction::I32Const(m_off as i32));
            f.instruction(&Instruction::I32Const(m_len as i32));
            f.instruction(&Instruction::ArrayNewData {
                array_type_index: T_BYTE_ARR,
                array_data_index: 1,
            });
            f.instruction(&Instruction::StructNew(T_STR));
            f.instruction(&Instruction::Call(fx::FN_FRAME_PUSH));
        }

        for item in &module.items {
            let mut ctx = EmitCtx {
                scan: &scan,
                res_maps: &res_maps,
                tab: &tab,
                strs: &scan.strings,
                buf_ptr: &mut buf_ptr,
                scope: &module_scope,
                depth: 0,
                br_continue: None,
                br_break: None,
                cls_obj: None,
                fg: g,
                eh_wrap_top: eh_wrap,
            };
            match item {
                Item::Stmt(stmt) => emit_stmt(&mut f, stmt, &mut ctx),
                // 03.03: module-level class definitions (Item::ClassDef) —
                // previously skipped here, leaving the class global null and
                // every reference to the class trapping downstream.
                Item::ClassDef(cd) => emit_class_def(&mut f, cd, &mut ctx),
                Item::FnDef(_) => {} // bound in the def-time loop above
            }
        }
        if eh_wrap {
            f.instruction(&Instruction::End); // try_table
            // success: deliver a null payload to $catch (the handler below
            // tests is_null — null = no exception)
            f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
            f.instruction(&Instruction::Br(0)); // -> $catch end
            f.instruction(&Instruction::End); // $catch: exc (or null) on stack
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Else);
            // exception: pending = payload; tb_render() to fd 2; rc 1
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
            f.instruction(&Instruction::GlobalSet(g.pending_exc));
            f.instruction(&Instruction::Call(fx::FN_TB_RENDER));
            f.instruction(&Instruction::Drop); // tb_render's i32 rc
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Call(PROC_EXIT_FUNC));
            f.instruction(&Instruction::End); // handler if
        } // eh_wrap
        f.instruction(&Instruction::End); // the _start fn body
        code.function(&f);
    }

    // ---- element segment (section id 9: after exports, before datacount)
    //      user-function dispatch entries for call_indirect ----
    let mut elems = ElementSection::new();
    let entries: Vec<u32> = (0..n_user).map(fn_idx).collect();
    if !entries.is_empty() {
        elems.segment(wasm_encoder::ElementSegment {
            mode: wasm_encoder::ElementMode::Active {
                table: Some(0),
                offset: &ConstExpr::i32_const(0),
            },
            elements: Elements::Functions(std::borrow::Cow::Owned(entries)),
        });
    }

    // ---- exports ----
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("_start", ExportKind::Func, fn_idx(n_user));
    exports.export("aprt_version", ExportKind::Global, G_RT_DONE);
    // ---- data-count (id 12; precedes code per S-07-43 order) ----
    let data_count = wasm_encoder::DataCountSection { count: 2 };

    // ---- data section (id 11, after code) ----
    // blob snapshot AFTER all emission (the _start EH prologue's late
    // tab.push calls must be included — see the NOTE above).
    let blob = tab.bytes.clone();
    let mut data = DataSection::new();
    data.segment(DataSegment {
        mode: DataSegmentMode::Active {
            memory_index: 0,
            offset: &ConstExpr::i32_const(prelude::SEG0_BASE),
        },
        data: blob.clone(),
    });
    data.segment(DataSegment {
        mode: DataSegmentMode::Passive,
        data: blob,
    });

    // ---- canonical section emission order (S-07-43) ----
    // tags (id 13, canonical slot between memory and globals; S-07-28)
    let mut tags = wasm_encoder::TagSection::new();
    tags.tag(wasm_encoder::TagType {
        kind: wasm_encoder::TagKind::Exception,
        func_type_idx: FT_TAG,
    });
    m.section(&build_type_section());
    m.section(&imports);
    m.section(&funcs);
    m.section(&tables);
    m.section(&mems);
    m.section(&tags);
    m.section(&globals);
    m.section(&exports);
    m.section(&elems);
    m.section(&data_count);
    m.section(&code);
    m.section(&data);

    // provenance custom section
    let mut custom: Vec<u8> = Vec::new();
    custom.push(12);
    custom.extend_from_slice(b"aprt-version");
    custom.push(b'0');
    m.section(&RawSection {
        id: wasm_encoder::SectionId::Custom as u8,
        data: &custom,
    });

    Ok(m.finish())
}

/// Function index of the proc_exit import (0 fd_write, 1 abort_msg, 2
/// proc_exit; rt_init and the prelude shift +1 accordingly).
const PROC_EXIT_FUNC: u32 = 2;
/// FT for proc_exit(i32) -> () — declared in types.rs.
const PROC_EXIT_FT: u32 = FT_PROC_EXIT;

fn mod_global(name: &str, module_names: &[String]) -> u32 {
    let i = module_names
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("scan registered every module name read: missing '{name}' (registered: {module_names:?})"));
    G_MOD_BASE + i as u32
}

// ===========================================================================
// rt_init (03.02a skeleton, carried forward verbatim)
// ===========================================================================

fn emit_rt_init_ordered(f: &mut Function) {
    let mut str_off = 0u32;
    let mut name_offsets: Vec<u32> = Vec::new();
    for name in TYPE_ORDER.iter().chain(EXC_TREE.iter().map(|(n, _)| n)) {
        name_offsets.push(str_off);
        str_off += name.len() as u32;
    }
    for (i, name) in TYPE_ORDER.iter().enumerate() {
        f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_TYPE,
        )));
        push_str_struct(f, name_offsets[i], name.len() as u32);
        // mro: every builtin type gets [self, object] (CPython: all builtins
        // derive object; issub walks this for isinstance/issubclass).
        let obj_root = *name == "object";
        let mro_len: i32 = if obj_root { 1 } else { 2 };
        f.instruction(&Instruction::I32Const(mro_len));
        f.instruction(&Instruction::ArrayNewDefault(T_TYPE_ARR));
        if mro_len > 0 {
            // mro[0] = self (patched after the struct exists; the array
            // is mutable), mro[1] = object for BaseException
            // (the self patch happens below, after GlobalSet)
        }
        f.instruction(&Instruction::I32Const(SLOT_COUNT as i32));
        f.instruction(&Instruction::ArrayNewDefault(T_SLOT_ARR));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
        push_empty_dict(f);
        f.instruction(&Instruction::StructNew(T_TYPE));
        let g = G_TYPE_BASE + i as u32;
        f.instruction(&Instruction::GlobalSet(g));
        if mro_len > 0 {
            // patch mro[0] = self; BaseException: mro[1] = object
            f.instruction(&Instruction::GlobalGet(g));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_TYPE,
                field_index: F_TYPE_MRO,
            });
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::GlobalGet(g));
            f.instruction(&Instruction::ArraySet(T_TYPE_ARR));
            if mro_len == 2 {
                f.instruction(&Instruction::GlobalGet(g));
                f.instruction(&Instruction::StructGet {
                    struct_type_index: T_TYPE,
                    field_index: F_TYPE_MRO,
                });
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::GlobalGet(
                    builtin_type_global("object").unwrap_or(0),
                ));
                f.instruction(&Instruction::ArraySet(T_TYPE_ARR));
            }
        }
    }

    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("NoneType").unwrap_or(0),
    ));
    f.instruction(&Instruction::StructNew(T_PYOBJ));
    f.instruction(&Instruction::GlobalSet(G_NONE));
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("ellipsis").unwrap_or(0),
    ));
    f.instruction(&Instruction::StructNew(T_PYOBJ));
    f.instruction(&Instruction::GlobalSet(G_ELLIPSIS));
    let bool_g = builtin_type_global("bool").unwrap_or(0);
    f.instruction(&Instruction::GlobalGet(bool_g));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_BIGINT,
    )));
    f.instruction(&Instruction::StructNew(T_INT));
    f.instruction(&Instruction::GlobalSet(G_TRUE));
    f.instruction(&Instruction::GlobalGet(bool_g));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_BIGINT,
    )));
    f.instruction(&Instruction::StructNew(T_INT));
    f.instruction(&Instruction::GlobalSet(G_FALSE));

    // ---- leaf 03.03: the builtin exception tree (S-03-32) ----
    // EXC_TREE rows in dependency order (rows referencing bases not yet
    // built are deferred — the table order isn't topological).
    let mut built: Vec<String> = TYPE_ORDER.iter().map(|n| n.to_string()).collect();
    let mut rows: std::collections::VecDeque<(String, String)> = EXC_TREE
        .iter()
        .map(|(n, b)| (n.to_string(), b.to_string()))
        .collect();
    // BaseException is in TYPE_ORDER — its EXC_TREE row is a no-op.
    let mut guard = 0usize;
    while let Some((name, base)) = rows.pop_front() {
        let name = name.as_str();
        let base = base.as_str();
        if name == "BaseException" {
            continue;
        }
        if !built.iter().any(|n| n == base) {
            rows.push_back((name.to_string(), base.to_string()));
            guard += 1;
            if guard > EXC_TREE.len() * EXC_TREE.len() {
                panic!("EXC_TREE base cycle");
            }
            continue;
        }
        guard = 0;
        // base $type ref
        let base_g = exc_type_global(base).expect("base built");
        f.instruction(&Instruction::GlobalGet(base_g));
        // name str (offsets computed over TYPE_ORDER + EXC_TREE names)
        let mut found: Option<(u32, u32)> = None;
        let mut acc = 0u32;
        for n in TYPE_ORDER.iter().chain(EXC_TREE.iter().map(|(n, _)| n)) {
            if *n == name {
                found = Some((acc, n.len() as u32));
                break;
            }
            acc += n.len() as u32;
        }
        let (off, len) = found.expect("exc name in the combined name blob");
        push_str_struct(f, off, len);
        // mro = [self] + base.mro — array.new_default + patch (mutable)
        f.instruction(&Instruction::GlobalGet(base_g));
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_TYPE,
            field_index: F_TYPE_MRO,
        });
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::ArrayNewDefault(T_TYPE_ARR));
        // slots / bases / dict
        f.instruction(&Instruction::I32Const(SLOT_COUNT as i32));
        f.instruction(&Instruction::ArrayNewDefault(T_SLOT_ARR));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
        push_empty_dict(f);
        f.instruction(&Instruction::StructNew(T_TYPE));
        let gi = exc_type_global(name).expect("exc row registered");
        f.instruction(&Instruction::GlobalSet(gi));
        // patch mro[0]=self, then mro[i+1]=base.mro[i] for all i (the copy
        // loop — the old single ArraySet clobbered mro[0] with base.mro[0],
        // leaving self out of its own mro so issub(cls, cls) was False)
        f.instruction(&Instruction::GlobalGet(gi));
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_TYPE,
            field_index: F_TYPE_MRO,
        });
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::GlobalGet(gi));
        f.instruction(&Instruction::ArraySet(T_TYPE_ARR));
        // copy loop: for i in 0..base.mro.len: self.mro[i+1] = base.mro[i].
        // Inline (rt_init has exactly one i32 local at index 0 — free
        // scratch; a new prelude fn would shift FN_FIRST_USER).
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(0));
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        // i >= base.mro.len? -> done (operands: [i, len] so GeU is i>=len)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::GlobalGet(base_g));
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_TYPE,
            field_index: F_TYPE_MRO,
        });
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // self.mro[i+1] = base.mro[i]
        f.instruction(&Instruction::GlobalGet(gi));
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_TYPE,
            field_index: F_TYPE_MRO,
        });
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::GlobalGet(base_g));
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_TYPE,
            field_index: F_TYPE_MRO,
        });
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::ArrayGet(T_TYPE_ARR));
        f.instruction(&Instruction::ArraySet(T_TYPE_ARR));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(0));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End); // loop
        f.instruction(&Instruction::End); // block
        f.instruction(&Instruction::GlobalGet(gi));
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_TYPE,
            field_index: F_TYPE_BASES,
        });
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::GlobalGet(base_g));
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
        built.push(name.to_string());
    }

    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::GlobalSet(G_RT_DONE));
}

fn push_str_struct(f: &mut Function, off: u32, len: u32) {
    // 03.03d FIX: stamp the str TYPE global (was RefNull(T_TYPE)) — these
    // name strings ride F_TYPE_NAME of every builtin/exception $type, and a
    // null type field made type(x).__name__ results fall through py_str's
    // type_sel to the null fallthrough (print trap).
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("str").unwrap_or(0),
    ));
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(len as i32));
    f.instruction(&Instruction::I32Const(off as i32));
    f.instruction(&Instruction::I32Const(len as i32));
    f.instruction(&Instruction::ArrayNewData {
        array_type_index: T_BYTE_ARR,
        array_data_index: 1,
    });
    f.instruction(&Instruction::StructNew(T_STR));
}

fn push_empty_dict(f: &mut Function) {
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_TYPE,
    )));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(7));
    f.instruction(&Instruction::I32Const(0));
    for _ in 0..8 {
        f.instruction(&Instruction::I32Const(-1));
    }
    f.instruction(&Instruction::ArrayNewFixed {
        array_type_index: T_IDX_ARR,
        array_size: 8,
    });
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayNewDefault(T_ENTRY_ARR));
    f.instruction(&Instruction::StructNew(T_DICT));
}

// ===========================================================================
// Function emission
// ===========================================================================

/// A scope: where names resolve during emission.
struct Scope {
    kind: ScopeKind,
    /// name -> resolution (module scope: empty — everything is Global).
    res: std::collections::HashMap<String, Res>,
    /// first cell-ref local index (params + plain locals come before).
    cell_base: u32,
    /// own-cell count (free cells start at cell_base + n_own).
    n_own: u32,
    #[allow(dead_code)]
    params: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum ScopeKind {
    Module,
    Fn,
}

/// Per-function emission context.
struct EmitCtx<'a> {
    scan: &'a Scan,
    #[allow(dead_code)]
    res_maps: &'a [std::collections::HashMap<String, Res>],
    #[allow(dead_code)]
    tab: &'a StrTab,
    strs: &'a [Vec<u8>],
    #[allow(dead_code)]
    buf_ptr: *mut i32,
    scope: &'a Scope,
    /// number of currently OPEN structured blocks (br-depth arithmetic).
    depth: u32,
    /// absolute depth of the innermost loop's continue/break targets.
    br_continue: Option<u32>,
    br_break: Option<u32>,
    /// Local slot of the class object under construction (03.03): method
    /// defs inside a class body attr_set onto it (S-03-3).
    cls_obj: Option<u32>,
    /// Frame/exception globals (03.03) — computed at emit time (they sit
    /// after the module namespace); the G_* consts are pre-namespace and
    /// wrong whenever the module has names.
    fg: crate::prelude_classes::FrameGlobals,
    /// Top-level EH wrap is active: the wrap's catch payload rides
    /// TEMP_LOCAL_A across the handler, so statement emission must NOT
    /// use TEMP_LOCAL_A as a scratch across block boundaries (03.03b).
    /// Scratch here = TEMP_LOCAL_B/TEMP_LOCAL_S.
    eh_wrap_top: bool,
}

/// Shared temp locals (valid in `_start` AND user-function frames; user
/// frames reserve the same indices at their local-block tail).
const TEMP_LOCAL: u32 = 40; // print-buffer end (i32)
#[allow(dead_code)]
const TEMP_LOCAL_I64: u32 = 41;
const TEMP_LOCAL_A: u32 = 42; // oref stash
const TEMP_LOCAL_B: u32 = 43;
const TEMP_LOCAL_OBJARR: u32 = 44; // mut $obj_arr stash
const TEMP_LOCAL_S: u32 = 45; // $str stash
const TEMP_LOCAL_CMP: u32 = 47; // compare spill (i32)
const TEMP_LOCAL_OBJARR_RO: u32 = 48; // RO array stash
#[allow(dead_code)]
const TEMP_LOCAL_FN: u32 = 49; // $function stash (reserved)
const TEMP_LOCAL_OBJARR2: u32 = 50; // second mut $obj_arr stash (param_names)
/// leaf 03.04c: iterator stash (the $generator iterator object across the
/// For/generator resume paths). _start and every user frame reserve 52.
const TEMP_LOCAL_IT: u32 = 52;


// ===========================================================================
// Generator lowering (leaf 03.04d): step-fn CFG flattener + call-site builds
// ===========================================================================
//
// DESIGN (NEXT-0304c item 1 + PROGRESS 03.04b design notes):
// A generator body flattens into STATES. Every top-level statement (and the
// STATEMENT AFTER every top-level yield) gets one state id. A `yield` parks
// at the state id of the NEXT statement. The step fn is:
//   Block $out
//     Block $done
//       try_table [catch TAG -> resume-label]   <- per-state EH wrap INSIDE
//         br_table(state) -> [state blocks]     <- all inside ONE try_table
//       (handler: staged-exn throw / normal-path $done)
//       Block s0 { <stmts of state 0>; br $done }  ... one Block per state,
//       emitted in order; br_table depth targets them OUTWARD.
//     (return-site: $value = None; state = DONE; return null)
//   ($out: return null — the dispatch exhausts)
// Resume = re-run the whole state block; loop condition re-tests ride the
// mirrors so iteration state survives. Control-flow bottom targets (While
// condition / For next-iteration) re-enter at the STATE containing the
// construct (its id), so `continue`/loop-back br to the STATE dispatch is
// impossible inside the block — they br to the construct's Loop label as
// usual. Yields INSIDE While/For/If bodies park at a state whose block
// re-enters the construct from its head: CPython-correct because the
// construct's own state (loop vars, iterator cursor) lives in mirrors/slots
// — EXCEPT the iterator object of a For-with-yield, which must ride a SLOT
// (local 52 dies at the suspension). Scan fail-closes that shape.
// ---------------------------------------------------------------------------

/// Per-generator emission plan, built at fn-emit time.
struct GenCtx {
    /// locals: 0 = $generator param, 1 = input param, 2 = state i32,
    /// 3 = payload stash (oref), 4..4+S = slot mirrors (oref),
    /// 4+S..4+S+8 = shared scratch (mirrors emit_user_fn's temp window:
    /// [A, B, OBJARR, S, I64, CMP-i32, ROARR, FN]).
    mirror_base: u32,
    scratch: GenScratch,
    /// Slot table: params then collect_local_binds (same filter as
    /// emit_user_fn's plain-local walk). Index = mirror position.
    slot_table: Vec<String>,
}

/// Absolute local indices of the shared scratch window inside step fns.
#[derive(Clone, Copy)]
struct GenScratch {
    a: u32,
    b: u32,
    objarr: u32,
    s: u32,
    i64v: u32,
    cmp: u32,
    roarr: u32,
    /// payload scratch (exc|null) for the next(x, default) guard.
    s_pad: u32,
    /// MUT $obj_arr temp (03.04f: $code.param_names build inside
    /// gen_emit_generator_new — scratch.objarr is busy holding $locals).
    objarr2: u32,
}

impl GenScratch {
    fn for_slots(n_slots: u32) -> Self {
        let base = 4 + n_slots;
        Self {
            a: base,
            b: base + 1,
            objarr: base + 2,
            s: base + 3,
            i64v: base + 4,
            cmp: base + 5,
            roarr: base + 6,
            s_pad: base + 7,
            objarr2: base + 8,
        }
    }
}

fn gen_slot_idx(gen: &GenCtx, name: &str) -> u32 {
    gen.mirror_base
        + gen.slot_table
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("generator slot table missing '{name}'")) as u32
}

/// `local.get 0; ref.cast (ref null $generator)` — cast-on-stack (lesson 4).
fn gen_lget_gen(f: &mut Function) {
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
}

/// Read a slot value onto the stack.
fn gen_load_slot(f: &mut Function, gen: &GenCtx, name: &str) {
    f.instruction(&Instruction::LocalGet(gen_slot_idx(gen, name)));
}

/// Store the stack-top into a slot.
fn gen_store_slot(f: &mut Function, gen: &GenCtx, name: &str) {
    f.instruction(&Instruction::LocalSet(gen_slot_idx(gen, name)));
}

/// The SEGMENT MACHINE plan (03.04e, NEXT-0304d design): the body lowers to
/// numbered SEGMENTS; $gen.$state = the resume segment id. Every segment's
/// wasm Block is OPEN during dispatch (nested Block(0..n-1) + br_table), so
/// a segment body can `br` OUTWARD to any LATER segment = forward goto.
/// Loop back-edges ride ONE dispatch Loop around the whole machine: a
/// back-edge sets the state register to the head segment and re-enters
/// dispatch (a `br` to the Loop label). Resume dispatch (a park's resume)
/// rides the same machine: $state already names the resume segment.
struct SegPlan {
    segs: Vec<Segment>,
    wrappers: Vec<WrapRange>,
    /// Loop-tail segment ids (while/for body ends): their fall-through is
    /// dead (the back-chain br + break/continue cover every exit) — the
    /// emitter unreachables it.
    loop_tails: Vec<SegId>,
}

#[derive(Clone, Debug)]
struct Segment {
    kind: SegKind,
    /// Override for the fall-through continuation (loop back-chain, handler
    /// resumes, then/else joins). None = physically next segment.
    next_override: Option<SegId>,
}

#[derive(Clone, Debug)]
enum SegKind {
    /// Straight-line statements (no yields — the planner splits at sites).
    Seq(Vec<Stmt>),
    /// If cond { goto yes } else { goto no }.
    Test { cond: Expr, yes: usize, no: usize },
    /// Suspend: $value = value_expr; state = next; spill; return value.
    Park {
        value_expr: Option<Expr>,
        next: usize,
        kind: ParkKind,
    },
    /// Resume side of Park Bind: slot = $gen.$value, then continue.
    Rebind { slot: String },
    /// Resume side of Park Split: slot = lhs OP $value, then continue.
    SplitResume {
        op: BinOp,
        lhs: Expr,
        slot: String,
    },
    /// For-entry: it_slot = iter_of(iter); fall through.
    ForInit { iter: Expr, it_slot: String },
    /// For-step: try iter_next(it_slot) -> bind target, fall through;
    /// SI -> goto exit.
    ForNext {
        target: String,
        it_slot: String,
        exit: usize,
    },
    /// yield-from step: next over it_slot; got x -> Park(x) TO SELF (inline);
    /// SI -> goto done (extracting the SI payload into scratch.a first).
    YfNext { it_slot: String, done: usize },
    /// yield-from completion: slot = scratch.a (the extracted SI payload =
    /// the delegate's return); fall through.
    YfDone { slot: String },
    /// Unconditional goto (forward br / backward trampoline).
    Goto(SegId),
    /// Handler-prefix statement group: runs inside the try wrapper close
    /// region (the handler just matched). When `park_value` is set the
    /// group yields (parking at `resume`, which carries the post stmts).
    HandlerPrefix {
        stmts: Vec<Stmt>,
        park_value: Option<Expr>,
        park_kind: ParkKind,
        resume: usize,
    },
    /// state = DONE; $value = value (None when bare); return null.
    Return { value: Option<Expr> },
}

#[derive(Clone, Debug, PartialEq)]
enum ParkKind {
    Bare,
    Bind(String),
    Split(BinOp, Expr, String),
}

/// SegId constants improve planner readability below.
type SegId = usize;

fn walk_stmt_yields(s: &Stmt) -> bool {
    match s {
        Stmt::ExprStmt(e) => ap_ir::walk::expr_yields(e),
        Stmt::Assign(_, v) => ap_ir::walk::expr_yields(v),
        Stmt::AugAssign(_, _, v) => ap_ir::walk::expr_yields(v),
        Stmt::Return(Some(e)) => ap_ir::walk::expr_yields(e),
        Stmt::If(c, a, b) => {
            ap_ir::walk::expr_yields(c)
                || a.iter().any(walk_stmt_yields)
                || b.iter().any(walk_stmt_yields)
        }
        Stmt::While(c, a, b) => {
            ap_ir::walk::expr_yields(c)
                || a.iter().any(walk_stmt_yields)
                || b.iter().any(walk_stmt_yields)
        }
        Stmt::For(_, it, a, b) => {
            ap_ir::walk::expr_yields(it)
                || a.iter().any(walk_stmt_yields)
                || b.iter().any(walk_stmt_yields)
        }
        Stmt::Try(ts) => {
            ts.body.iter().any(walk_stmt_yields)
                || ts.handlers.iter().any(|h| h.body.iter().any(walk_stmt_yields))
                || ts.orelse.iter().any(walk_stmt_yields)
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// 03.04e segment-machine planner
// ---------------------------------------------------------------------------

/// One planned try/except wrapper: the segment range [inner0..after) runs
/// with a per-segment try_table re-open carrying `handlers`.
struct WrapRange {
    inner0: usize,
    after: usize,
    handlers: Vec<HandlerPlan>,
}

/// One except clause. `prefix` runs inline in the wrapper close region; a
/// handler with a yield parks at `resume` (the post-park continuation
/// segment, placed after the wrapper range — its Seq carries `post`).
struct HandlerPlan {
    ty: Option<Expr>,
    name: Option<String>,
    prefix: Vec<Stmt>,
    park_value: Option<Expr>,
    resume: Option<SegId>,
    /// staged HandlerPrefix segment (park-yielding handlers only).
    prefix_seg: Option<SegId>,
}

/// Continuation sentinels staged in `Segment::next_override` during
/// planning (resolved by the enclosing loop pass or at plan end):
/// E_PEND = fell off the current statement stream (= continue to the loop
/// top / done escape), BRK_PEND = break, CONT_PEND = explicit continue.
const E_PEND: SegId = usize::MAX;
const BRK_PEND: SegId = usize::MAX - 1;
const CONT_PEND: SegId = usize::MAX - 2;

/// Planner state (03.04e segment machine).
struct SegPlanner {
    segs: Vec<Segment>,
    wrappers: Vec<WrapRange>,
    for_count: usize,
    loop_depth: usize,
    loop_tails: Vec<SegId>,
    /// Handler-prefix segments staged at plan time (wired at plan end):
    /// (prefix_seg, resume_seg, prefix_stmts, park_value, park_kind).
    pending_prefix: Vec<(SegId, Option<SegId>, Vec<Stmt>, Option<Expr>, ParkKind)>,
}

const YF_IT_SLOT: &str = "__ap_yf_it";

fn fail_closed_gen(why: &str) -> ! {
    panic!("generator segment planner fail-closed: {why}")
}

/// Split a handler body at its first yield-carrying statement: (prefix,
/// the yield VALUE if the split stmt is a top-level ExprStmt(Yield), post).
/// A yield nested deeper (inside If/While in a handler) fail-closes.
fn split_handler_body(body: &[Stmt]) -> (Vec<Stmt>, Option<Expr>, Vec<Stmt>) {
    for (i, s) in body.iter().enumerate() {
        let hit = match s {
            Stmt::ExprStmt(Expr::Yield(v)) => Some(v.as_ref().map(|x| x.as_ref().clone())),
            Stmt::Assign(_, v) if matches!(v, Expr::Yield(_)) => {
                fail_closed_gen("handler bind-yield beyond the slice");
            }
            other if walk_stmt_yields(other) => {
                fail_closed_gen("yield nested in a handler beyond the slice");
            }
            _ => None,
        };
        if let Some(yv) = hit {
            return (body[..i].to_vec(), yv, body[i + 1..].to_vec());
        }
    }
    (body.to_vec(), None, Vec::new())
}

/// Hidden suspension-surviving slots a generator body needs, in the SAME
/// pre-order the planner allocates them: one it-slot per For statement, the
/// shared yield-from it-slot. Call sites (gen_emit_generator_new /
/// emit_generator_call) size $locals with params + local binds + THIS list.
fn gen_hidden_slots(body: &[Stmt]) -> Vec<String> {
    fn walk(stmts: &[Stmt], out: &mut Vec<String>, n_for: &mut usize) {
        for s in stmts {
            match s {
                Stmt::For(_target, _iter, b, _els) => {
                    let slot = format!("__ap_for_it{}", *n_for);
                    *n_for += 1;
                    out.push(slot);
                    walk(b, out, n_for);
                }
                Stmt::If(_, a, b) => {
                    walk(a, out, n_for);
                    walk(b, out, n_for);
                }
                Stmt::While(_, b, els) => {
                    walk(b, out, n_for);
                    walk(els, out, n_for);
                }
                Stmt::Try(ts) => {
                    walk(&ts.body, out, n_for);
                    for h in &ts.handlers {
                        walk(&h.body, out, n_for);
                    }
                    walk(&ts.orelse, out, n_for);
                }
                Stmt::ExprStmt(Expr::YieldFrom(_)) | Stmt::Assign(_, Expr::YieldFrom(_)) => {
                    if !out.iter().any(|s| s == YF_IT_SLOT) {
                        out.push(YF_IT_SLOT.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    let mut n_for = 0usize;
    walk(body, &mut out, &mut n_for);
    out
}

impl SegPlanner {
    fn push(&mut self, kind: SegKind) -> SegId {
        self.segs.push(Segment {
            kind,
            next_override: None,
        });
        self.segs.len() - 1
    }

    fn patch(&mut self, id: SegId, kind: SegKind) {
        self.segs[id].kind = kind;
    }

    /// Plan a statement list; returns (first, last, end_position). `chain`
    /// overrides the LAST segment's fall-through (loop bodies chain to the
    /// loop top).
    fn plan_seq(&mut self, stmts: &[Stmt], chain: Option<SegId>) -> (SegId, SegId, SegId) {
        let mut first: Option<SegId> = None;
        let mut last: SegId = 0;
        for s in stmts {
            let (f, l) = self.plan_stmt(s);
            if first.is_none() {
                first = Some(f);
            }
            last = l;
        }
        let first = first.unwrap_or_else(|| self.push(SegKind::Seq(Vec::new())));
        let end = self.segs.len();
        if let Some(c) = chain {
            self.segs[last].next_override = Some(c);
        }
        (first, last, end)
    }

    /// Patch sentinels created while planning the segment range
    /// [start..end): E_PEND/CONT_PEND -> `top`, BRK_PEND -> `after`.
    fn patch_loop_sentinels(&mut self, start: SegId, end: SegId, top: SegId, after: SegId) {
        for i in start..end {
            match self.segs[i].next_override {
                Some(E_PEND) | Some(CONT_PEND) => self.segs[i].next_override = Some(top),
                Some(BRK_PEND) => self.segs[i].next_override = Some(after),
                _ => {}
            }
        }
    }

    fn plan_stmt(&mut self, s: &Stmt) -> (SegId, SegId) {
        match s {
            Stmt::Line(_) | Stmt::Pass | Stmt::Global(_) | Stmt::Nonlocal(_) => {
                let id = self.push(SegKind::Seq(Vec::new()));
                (id, id)
            }
            Stmt::ExprStmt(e) => {
                if let Expr::Yield(v) = e {
                    let pk = self.push(SegKind::Park {
                        value_expr: v.as_ref().map(|x| x.as_ref().clone()),
                        next: self.segs.len() + 1,
                        kind: ParkKind::Bare,
                    });
                    return (pk, pk); // resume = physically next segment
                }
                if let Expr::YieldFrom(x) = e {
                    let id = self.plan_yield_from(x.as_ref().clone(), None);
                    return (id, id);
                }
                if ap_ir::walk::expr_yields(e) {
                    fail_closed_gen("yield nested beyond a top-level ExprStmt");
                }
                let id = self.push(SegKind::Seq(vec![s.clone()]));
                (id, id)
            }
            Stmt::Assign(targets, value) => {
                let ap_ir::item::Target::Name(tname) = &targets[0] else {
                    fail_closed_gen("generator assign target beyond a name");
                };
                let slot = tname.as_str().to_string();
                match value {
                    Expr::Yield(v) => {
                        let pk = self.push(SegKind::Park {
                            value_expr: v.as_ref().map(|x| x.as_ref().clone()),
                            next: self.segs.len() + 1,
                            kind: ParkKind::Bind(slot.clone()),
                        });
                        let rb = self.push(SegKind::Rebind { slot });
                        (pk, rb)
                    }
                    Expr::YieldFrom(x) => {
                        let id = self.plan_yield_from(x.as_ref().clone(), Some(slot));
                        (id, id)
                    }
                    Expr::BinOp(op, lhs, rhs) if matches!(rhs.as_ref(), Expr::Yield(_)) => {
                        // GE002 shape: slot = lhs OP (yield v). The Park
                        // carries the split; the resume segment finishes the
                        // op. lhs must be a Name — re-read from the spilled
                        // mirror at resume (NO expression-level yield eval).
                        let Expr::Yield(yv) = rhs.as_ref() else {
                            unreachable!()
                        };
                        if !matches!(lhs.as_ref(), Expr::Name(_)) {
                            fail_closed_gen("split-yield lhs beyond a name");
                        }
                        let pk = self.push(SegKind::Park {
                            value_expr: yv.as_ref().map(|x| x.as_ref().clone()),
                            next: self.segs.len() + 1,
                            kind: ParkKind::Split(*op, lhs.as_ref().clone(), slot.clone()),
                        });
                        let rb = self.push(SegKind::SplitResume {
                            op: *op,
                            lhs: lhs.as_ref().clone(),
                            slot,
                        });
                        (pk, rb)
                    }
                    other => {
                        if ap_ir::walk::expr_yields(other) {
                            fail_closed_gen("yield nested in assign value beyond the slice");
                        }
                        let id = self.push(SegKind::Seq(vec![s.clone()]));
                        (id, id)
                    }
                }
            }
            Stmt::AugAssign(..) => {
                let id = self.push(SegKind::Seq(vec![s.clone()]));
                (id, id)
            }
            Stmt::If(cond, then, els) => {
                if ap_ir::walk::expr_yields(cond) {
                    fail_closed_gen("yield in a gen if-condition");
                }
                let t = self.push(SegKind::Test {
                    cond: cond.clone(),
                    yes: 0,
                    no: 0,
                });
                let (tf, tl, _te) = self.plan_seq(then, None);
                let (ef, el, _ee) = self.plan_seq(els, None);
                // then-tail continues AFTER the whole if: stage E_PEND — the
                // enclosing loop pass (-> loop top) or plan end (-> done
                // escape) resolves it.
                self.segs[tl].next_override = Some(E_PEND);
                self.patch(
                    t,
                    SegKind::Test {
                        cond: cond.clone(),
                        yes: tf,
                        no: ef,
                    },
                );
                (t, el.max(tl))
            }
            Stmt::While(cond, body, els) => {
                if ap_ir::walk::expr_yields(cond) || !els.is_empty() {
                    fail_closed_gen("yield in gen while-condition / while-else");
                }
                let head = self.push(SegKind::Test {
                    cond: cond.clone(),
                    yes: 0,
                    no: 0,
                });
                let body_start = self.segs.len();
                self.loop_depth += 1;
                let (_b0, bl, body_end) = self.plan_seq(body, None);
                self.loop_depth -= 1;
                let after = self.segs.len();
                self.patch_loop_sentinels(body_start, body_end, head, after);
                self.segs[bl].next_override = Some(head); // loop back-chain
                self.loop_tails.push(bl);
                self.patch(
                    head,
                    SegKind::Test {
                        cond: cond.clone(),
                        yes: body_start,
                        no: after,
                    },
                );
                (head, bl)
            }
            Stmt::Break => {
                let id = self.push(SegKind::Goto(BRK_PEND));
                self.segs[id].next_override = Some(BRK_PEND);
                (id, id)
            }
            Stmt::Continue => {
                let id = self.push(SegKind::Goto(CONT_PEND));
                self.segs[id].next_override = Some(CONT_PEND);
                (id, id)
            }
            Stmt::For(target, iter, body, els) => {
                if ap_ir::walk::expr_yields(iter) || !els.is_empty() {
                    fail_closed_gen("yield in gen for-iter / for-else");
                }
                let ap_ir::item::Target::Name(tname) = target else {
                    fail_closed_gen("gen for target beyond a name");
                };
                let it_slot = format!("__ap_for_it{}", self.for_count);
                self.for_count += 1;
                let fi = self.push(SegKind::ForInit {
                    iter: iter.clone(),
                    it_slot: it_slot.clone(),
                });
                let fnx = self.push(SegKind::ForNext {
                    target: tname.as_str().to_string(),
                    it_slot: it_slot.clone(),
                    exit: 0,
                });
                let body_start = self.segs.len();
                let (_b0, bl, body_end) = self.plan_seq(body, None);
                let after = self.segs.len();
                self.patch_loop_sentinels(body_start, body_end, fnx, after);
                self.segs[bl].next_override = Some(fnx); // next iteration
                self.loop_tails.push(bl);
                self.patch(
                    fnx,
                    SegKind::ForNext {
                        target: tname.as_str().to_string(),
                        it_slot,
                        exit: after,
                    },
                );
                (fi, bl)
            }
            Stmt::Try(ts) => {
                if !ts.orelse.is_empty() || !ts.finalbody.is_empty() {
                    fail_closed_gen("gen try orelse/finally");
                }
                let w0 = self.segs.len();
                // 1. try body segments (each re-wrapped per-segment at emit)
                let (_b0, _bl, _be) = self.plan_seq(&ts.body, None);
                let after = self.segs.len();
                // 2. handler plan: split at the yield; prefix runs inline
                // in the wrapper close region; a yielded prefix parks at
                // `resume` (the post segment, placed after the wrapper).
                let mut handlers = Vec::new();
                for h in &ts.handlers {
                    let (prefix, park_value, post) = split_handler_body(&h.body);
                    let mut pid: Option<SegId> = None;
                    let resume = if park_value.is_some() {
                        let rid = self.push(SegKind::Seq(post));
                        self.segs[rid].next_override = Some(E_PEND);
                        // prefix segment staged now, wired at plan end
                        let p = self.push(SegKind::Seq(Vec::new()));
                        pid = Some(p);
                        let pk = match &h.name {
                            Some(n) => ParkKind::Bind(n.as_str().to_string()),
                            None => ParkKind::Bare,
                        };
                        self.pending_prefix.push((
                            pid.unwrap(),
                            Some(rid),
                            prefix.clone(),
                            park_value.clone(),
                            pk,
                        ));
                        Some(rid)
                    } else {
                        None
                    };
                    let prefix_seg = pid;
                    handlers.push(HandlerPlan {
                        ty: h.ty.clone(),
                        name: h.name.clone(),
                        prefix,
                        park_value,
                        resume,
                        prefix_seg,
                    });
                }
                self.wrappers.push(WrapRange {
                    inner0: w0,
                    after,
                    handlers,
                });
                let last = after - 1;
                (w0, last)
            }
            Stmt::Return(e) => {
                if e.as_ref().is_some_and(|x| ap_ir::walk::expr_yields(x)) {
                    fail_closed_gen("yield in gen return");
                }
                let id = self.push(SegKind::Return { value: e.clone() });
                (id, id)
            }
            Stmt::Raise(..) => {
                let id = self.push(SegKind::Seq(vec![s.clone()]));
                (id, id)
            }
            other => fail_closed_gen(&format!("gen stmt beyond the segment slice: {other:?}")),
        }
    }

    /// `yield from e` [ -> bind slot ]: ForInit(it) + YfNext (park-to-self)
    /// + YfDone (binds the extracted SI payload) or an empty Seq.
    fn plan_yield_from(&mut self, x: Expr, bind: Option<String>) -> SegId {
        let fi = self.push(SegKind::ForInit {
            iter: x,
            it_slot: YF_IT_SLOT.to_string(),
        });
        let yn = self.push(SegKind::YfNext {
            it_slot: YF_IT_SLOT.to_string(),
            done: 0,
        });
        let yd = match bind {
            Some(slot) => self.push(SegKind::YfDone { slot }),
            None => self.push(SegKind::Seq(Vec::new())),
        };
        self.patch(
            yn,
            SegKind::YfNext {
                it_slot: YF_IT_SLOT.to_string(),
                done: yd,
            },
        );
        yn
    }
}

/// Plan a generator body into the segment machine.
fn gen_plan_segments(uf: &UserFn) -> SegPlan {
    let mut p = SegPlanner {
        segs: Vec::new(),
        wrappers: Vec::new(),
        for_count: 0,
        loop_depth: 0,
        loop_tails: Vec::new(),
        pending_prefix: Vec::new(),
    };
    let (_f, _l, _e) = p.plan_seq(&uf.body, None);
    let n = p.segs.len();
    // resolve top-level sentinels: E_PEND -> n (= the done escape); loop
    // sentinels were already resolved by their loop passes.
    for seg in p.segs.iter_mut() {
        match seg.next_override {
            Some(E_PEND) => seg.next_override = Some(n),
            Some(BRK_PEND) | Some(CONT_PEND) => {
                fail_closed_gen("break/continue outside a gen loop")
            }
            _ => {}
        }
    }
    // wire handler-prefix segments (their ids are final now)
    for (pid, resume, stmts, park_value, pk) in p.pending_prefix.drain(..) {
        p.segs[pid].kind = SegKind::HandlerPrefix {
            stmts,
            park_value,
            park_kind: pk,
            resume: resume.unwrap(),
        };
    }
    SegPlan {
        segs: p.segs,
        wrappers: p.wrappers,
        loop_tails: p.loop_tails,
    }
}

/// Emit one generator step fn (03.04e): the SEGMENT MACHINE.
///
/// Layout:
///   (prologue: reload mirrors; state -> local 2)
///   Loop                    <- the dispatch trampoline
///     Block $done
///       Block(n-1)..Block(0) { br_table(local 2) } <seg0> ... <seg n-1>
///       (n default arm lands here = done escape)
///       $value = None; state = DONE; return null        <- done region
///     End ($done)
///   End (Loop)
///   RefNull (FT filler); End (fn body)
///
/// Every segment's Block is OPEN while its body runs, so a segment body can
/// `br` OUTWARD to any LATER segment's label = forward goto. Back-edges
/// (loop continue) set the state register and `br` to the Loop label (the
/// trampoline re-enters dispatch). A Park's resume rides the machine too:
/// $gen.$state names the resume segment; gen_send calls the step fn again.
fn emit_gen_step_fn(f: &mut Function, uf: &UserFn, ctx: &mut EmitCtx) {
    let n_slots = {
        let mut t: Vec<String> = uf.params.iter().map(|p| p.name.to_string()).collect();
        for n in collect_local_binds(&uf.body) {
            if uf.params.iter().any(|p| p.name == n)
                || uf.own_cells.contains(&n)
                || uf.free.contains(&n)
                || uf.free_cells.contains(&n)
            {
                continue;
            }
            if !t.contains(&n) {
                t.push(n);
            }
        }
        // hidden suspension-surviving slots MUST land in the same order
        // gen_slot_count appends them (the $locals sizing must match).
        let mut hidden = gen_hidden_slots(&uf.body);
        t.append(&mut hidden);
        t
    };
    let gen = GenCtx {
        mirror_base: 4,
        scratch: GenScratch::for_slots(n_slots.len() as u32),
        slot_table: n_slots.clone(),
    };
    let plan = gen_plan_segments(uf);
    let n_segs = plan.segs.len();
    let loop_tails: std::collections::HashSet<usize> =
        plan.loop_tails.iter().copied().collect();
    // try-wrapper map: wrapper id covering segment k (inner0 <= k < after).
    // More than one nested wrapper over one segment is not planned.
    let mut wrap_of: Vec<Option<usize>> = vec![None; n_segs];
    for (wid, w) in plan.wrappers.iter().enumerate() {
        for k in w.inner0..w.after {
            wrap_of[k] = Some(wid);
        }
    }
    // per-segment staged HandlerPrefix seg (first park-yield handler)
    let mut prefix_seg_of: Vec<Option<usize>> = vec![None; n_segs];
    for (wid, w) in plan.wrappers.iter().enumerate() {
        let pseg = w
            .handlers
            .iter()
            .find_map(|h| h.prefix_seg);
        for k in w.inner0..w.after {
            prefix_seg_of[k] = pseg;
        }
    }

    // 1. reload ALL mirrors from $gen.$locals (params land here at GS_START;
    // later resumes re-load the spilled values).
    for i in 0..gen.slot_table.len() as u32 {
        gen_lget_gen(f);
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_GENERATOR,
            field_index: F_GEN_LOCALS,
        });
        f.instruction(&Instruction::I32Const(i as i32));
        f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
        f.instruction(&Instruction::LocalSet(gen.mirror_base + i));
    }
    // 2. state scratch = $gen.$state -> local 2
    gen_lget_gen(f);
    sget(f, T_GENERATOR, F_GEN_STATE);
    f.instruction(&Instruction::LocalSet(2));

    // 3. the dispatch trampoline
    f.instruction(&Instruction::Loop(BlockType::Empty));
    f.instruction(&Instruction::Block(BlockType::Empty)); // $done
    f.instruction(&Instruction::Block(BlockType::Empty)); // $sink
    for _ in (0..n_segs).rev() {
        f.instruction(&Instruction::Block(BlockType::Empty)); // Block si
    }
    {
        let targets: Vec<u32> = (0..n_segs as u32).collect();
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::BrTable(
            std::borrow::Cow::Owned(targets),
            n_segs as u32, // default = done escape
        ));
    }
    // segment bodies (label math: inside segment k's body the open labels
    // are [seg blocks k..n-1] (depths 1..n-k from inside k), $done = n-k+1,
    // Loop = n-k+2; a `br d` with d = target+k+1 (target > k) lands at
    // segment target; d = n-k+1 -> $done; d = n-k+2 -> Loop re-dispatch.)
    for k in 0..n_segs {
        f.instruction(&Instruction::End); // close Block k
        let seg = &plan.segs[k];
        // fall-through continuation: next_override or k+1 (k+1 == n at the
        // tail = the done escape via br_table default).
        let next = seg.next_override.unwrap_or(k + 1);
        let dead_tail = loop_tails.contains(&k);
        // Continuation emitter. Open labels at segment k's BODY level
        // (innermost first): B(k)=0, B(k+1)=1, ..., B(n-1)=n-k-1,
        // $sink=n-k, $done=n-k+1, Loop=n-k+2.
        //   forward goto t (k < t < n): br (t - k - 1)  -> B(t-1) end ->
        //     seg t body (End(B j) is physically followed by seg j+1's body)
        //   fall-through (t = k+1, extra = 0): fall off (B(k) end -> next)
        //   done escape (t >= n):       br (n - k)      -> $sink end ->
        //     the done region (which sits between End($sink) and End($done))
        //   trampoline (t <= k):        state reg + br (n - k + 2) -> Loop
        // `extra` = labels opened at the branch site beyond body level
        // (If = +1, $chk-Block + If = +2): every depth += extra.
        let goto_x = |f: &mut Function, target: usize, extra: usize| {
            if target == k + 1 {
                // physical fall-through at body level; from INSIDE a wrapper
                // a fall-through would run the wrong region — trampoline.
                if extra == 0 {
                    return;
                }
                f.instruction(&Instruction::I32Const((k + 1) as i32));
                f.instruction(&Instruction::LocalSet(2));
                f.instruction(&Instruction::Br((n_segs - k + 1 + extra) as u32));
            } else if target >= n_segs {
                f.instruction(&Instruction::Br((n_segs - k + extra) as u32));
            } else if target > k {
                f.instruction(&Instruction::Br((target - k - 1 + extra) as u32));
            } else {
                f.instruction(&Instruction::I32Const(target as i32));
                f.instruction(&Instruction::LocalSet(2));
                f.instruction(&Instruction::Br((n_segs - k + 1 + extra) as u32));
            }
        };
        let wrap_extra: usize = if wrap_of[k].is_some() { 2 } else { 0 };
        let mut goto = move |f: &mut Function, target: usize| {
            goto_x(f, target, wrap_extra);
        };
        // ---- per-segment try_table re-open (03.04e GE003) ----
        let in_wrap = wrap_of[k].is_some();
        if in_wrap {
            f.instruction(&Instruction::Block(BlockType::Result(
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_EXCEPTION),
                }),
            ))); // $cx
            let catches = [wasm_encoder::Catch::One {
                tag: TAG_PYEXC,
                label: 0,
            }];
            f.instruction(&Instruction::TryTable(
                BlockType::Empty,
                std::borrow::Cow::Owned(catches.to_vec()),
            ));
        }
        match &seg.kind {
            SegKind::Seq(stmts) => {
                for s in stmts {
                    gen_emit_stmt(f, s, ctx, &gen);
                }
                // loop tail: the back-chain goto ALWAYS emits (a backward
                // target = state reg + Loop br). No dead-fall special case
                // needed — the br routes every arrival back to the head.
                goto(f, next);
            }
            SegKind::Test { cond, yes, no } => {
                // cond ON the ground BEFORE the If: the compare chain reads
                // (int-from-i64 + bool build) need the module-frame locals —
                // inside an If those allocs wedged wasmtime in 03.02c (the
                // While emitter note); the same hazard applies here.
                gen_emit_cond(f, cond, ctx, &gen);
                f.instruction(&Instruction::LocalSet(gen.scratch.cmp));
                f.instruction(&Instruction::LocalGet(gen.scratch.cmp));
                f.instruction(&Instruction::If(BlockType::Empty));
                // inside the If: +1 label
                goto_x(f, *yes, 1 + wrap_extra);
                f.instruction(&Instruction::Else);
                goto_x(f, *no, 1 + wrap_extra);
                f.instruction(&Instruction::End); // If
                // both arms br'd away; unreachable-tail the fall-through so
                // no half-truthy state can ever leak into the next segment.
                f.instruction(&Instruction::Unreachable);
            }
            SegKind::Park {
                value_expr,
                next: park_next,
                kind,
            } => {
                match value_expr {
                    Some(x) => gen_emit_expr(f, x, ctx, &gen),
                    None => {
                        f.instruction(&Instruction::GlobalGet(G_NONE));
                    }
                }
                match kind {
                    ParkKind::Bare => {}
                    ParkKind::Bind(_slot) => {} // Rebind segment does the bind
                    ParkKind::Split(_op, _lhs, _slot) => {}
                }
                gen_park_yield(f, &gen, *park_next as i32);
                // control left the fn (park returned); NOTHING after — not
                // even a validator-necessary br: the fn-end is implicit.
            }
            SegKind::Rebind { slot } => {
                gen_lget_gen(f);
                sget(f, T_GENERATOR, F_GEN_VALUE);
                gen_store_slot(f, &gen, slot.as_str());
                goto(f, next);
            }
            SegKind::SplitResume { op, lhs, slot } => {
                f.instruction(&Instruction::I32Const(binop_op(*op)));
                gen_emit_expr(f, lhs, ctx, &gen);
                gen_lget_gen(f);
                sget(f, T_GENERATOR, F_GEN_VALUE);
                f.instruction(&Instruction::Call(fx::FN_BINOP));
                gen_store_slot(f, &gen, slot.as_str());
                goto(f, next);
            }
            SegKind::ForInit { iter, it_slot } => {
                // 03.04f: the iter expr self-materializes — a generator
                // delegate (`sub()`) now builds the parked $generator via the
                // static gen-call arm, a list literal builds the list.
                // call_of(gen) had NO matching arm (gen is neither $function,
                // $type, nor instance) and fell into its unreachable — trap
                // on the first delegate resume.
                gen_emit_expr(f, iter, ctx, &gen);
                f.instruction(&Instruction::Call(fx::FN_ITER_OF));
                gen_store_slot(f, &gen, it_slot.as_str());
                goto(f, next);
            }
            SegKind::ForNext {
                target,
                it_slot,
                exit,
            } => {
                // x = iter_next(it) guarded by try_table (SI -> exit; other
                // raises rethrow)
                f.instruction(&Instruction::Block(BlockType::Result(
                    ValType::Ref(wasm_encoder::RefType {
                        nullable: true,
                        heap_type: hc(T_EXCEPTION),
                    }),
                ))); // $cx
                {
                    let catches = [wasm_encoder::Catch::One {
                        tag: TAG_PYEXC,
                        label: 0,
                    }];
                    f.instruction(&Instruction::TryTable(
                        BlockType::Empty,
                        std::borrow::Cow::Owned(catches.to_vec()),
                    ));
                    gen_load_slot(f, &gen, it_slot.as_str());
                    f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
                    // consume the result INSIDE (BlockType::Empty ends
                    // stack-empty; the catch replaces the stack anyway)
                    f.instruction(&Instruction::LocalSet(gen.scratch.a));
                    f.instruction(&Instruction::End); // try_table
                    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
                }
                f.instruction(&Instruction::End); // $cx: exc|null on stack
                f.instruction(&Instruction::LocalSet(gen.scratch.b));
                f.instruction(&Instruction::Block(BlockType::Empty)); // $chk
                {
                    // null -> success: fall through to the bind
                    f.instruction(&Instruction::LocalGet(gen.scratch.b));
                    f.instruction(&Instruction::RefIsNull);
                    f.instruction(&Instruction::If(BlockType::Empty));
                    f.instruction(&Instruction::Br(1));
                    f.instruction(&Instruction::End);
                    // SI -> exhaustion: goto exit
                    f.instruction(&Instruction::LocalGet(gen.scratch.b));
                    f.instruction(&Instruction::GlobalGet(
                        crate::types::exc_type_global("StopIteration").unwrap_or(0),
                    ));
                    f.instruction(&Instruction::Call(fx::FN_EXC_MATCH));
                    f.instruction(&Instruction::If(BlockType::Empty));
                    goto_x(f, *exit, 2 + wrap_extra);
                    f.instruction(&Instruction::End);
                    // other raise: rethrow
                    f.instruction(&Instruction::LocalGet(gen.scratch.b));
                    f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
                    f.instruction(&Instruction::Throw(TAG_PYEXC));
                }
                f.instruction(&Instruction::End); // $chk: success falls here
                f.instruction(&Instruction::LocalGet(gen.scratch.a));
                gen_store_slot(f, &gen, target.as_str());
                goto(f, next);
            }
            SegKind::YfNext { it_slot, done } => {
                // x = iter_next(it) guarded; SI -> stash payload -> done;
                // other raise -> rethrow; got x -> yield x (park to SELF).
                f.instruction(&Instruction::Block(BlockType::Result(
                    ValType::Ref(wasm_encoder::RefType {
                        nullable: true,
                        heap_type: hc(T_EXCEPTION),
                    }),
                ))); // $cx
                {
                    let catches = [wasm_encoder::Catch::One {
                        tag: TAG_PYEXC,
                        label: 0,
                    }];
                    f.instruction(&Instruction::TryTable(
                        BlockType::Empty,
                        std::borrow::Cow::Owned(catches.to_vec()),
                    ));
                    gen_load_slot(f, &gen, it_slot.as_str());
                    f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
                    // consume INSIDE (empty-stack at try_table End)
                    f.instruction(&Instruction::LocalSet(gen.scratch.a));
                    f.instruction(&Instruction::End); // try_table
                    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
                }
                f.instruction(&Instruction::End); // $cx
                f.instruction(&Instruction::LocalSet(gen.scratch.b));
                f.instruction(&Instruction::Block(BlockType::Empty)); // $chk
                {
                    f.instruction(&Instruction::LocalGet(gen.scratch.b));
                    f.instruction(&Instruction::RefIsNull);
                    f.instruction(&Instruction::If(BlockType::Empty));
                    f.instruction(&Instruction::Br(1));
                    f.instruction(&Instruction::End);
                    // SI: extract args[0] (or None) into scratch.b for
                    // the YfDone segment, then goto done
                    f.instruction(&Instruction::LocalGet(gen.scratch.b));
                    f.instruction(&Instruction::GlobalGet(
                        crate::types::exc_type_global("StopIteration").unwrap_or(0),
                    ));
                    f.instruction(&Instruction::Call(fx::FN_EXC_MATCH));
                    f.instruction(&Instruction::If(BlockType::Empty));
                    // exhaustion: extract the SI payload (args[0] or None)
                    // into scratch.b for the YfDone segment. The CAUGHT exc
                    // rides scratch.b ($cx result) — the SI-match branch
                    // must read IT, not scratch.a (which only holds the
                    // success-path result and is null on the raise path —
                    // 03.04f: struct.get on the null cast trapped here).
                    f.instruction(&Instruction::LocalGet(gen.scratch.b));
                    f.instruction(&Instruction::LocalSet(gen.scratch.a));
                    gen_extract_si_payload(f, &gen);
                    goto_x(f, *done, 2 + wrap_extra);
                    f.instruction(&Instruction::End);
                    f.instruction(&Instruction::LocalGet(gen.scratch.b));
                    f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
                    f.instruction(&Instruction::Throw(TAG_PYEXC));
                }
                f.instruction(&Instruction::End); // $chk: success -> scratch.a = x
                // got x: yield it — park TO SELF (resume re-enters this
                // segment; the it-slot carries the delegate cursor).
                f.instruction(&Instruction::LocalGet(gen.scratch.a));
                gen_park_yield(f, &gen, k as i32);
            }
            SegKind::YfDone { slot } => {
                // the delegate's return rides scratch.b (stashed by the
                // YfNext SI arm): bind it.
                f.instruction(&Instruction::LocalGet(gen.scratch.b));
                gen_store_slot(f, &gen, slot.as_str());
                goto(f, next);
            }
            SegKind::Goto(target) => {
                goto(f, *target);
            }
            SegKind::HandlerPrefix {
                stmts,
                park_value,
                park_kind,
                resume,
            } => {
                let _ = park_kind; // the `as n` bind rides gen_store in the
                // resume segment via Park Bind? Simpler: bind HERE from the
                // payload when park lands. The park's $value = yield expr;
                // on resume gen_send parks the SENT value; the handler name
                // binds the EXC — bound in the wrapper chain (s_pad), not
                // from $value. So no Bind handling here: plain park.
                for s in stmts {
                    gen_emit_stmt(f, s, ctx, &gen);
                }
                match park_value {
                    Some(x) => gen_emit_expr(f, x, ctx, &gen),
                    None => {
                        f.instruction(&Instruction::GlobalGet(G_NONE));
                    }
                }
                gen_park_yield(f, &gen, *resume as i32);
            }
            SegKind::Return { value } => {
                match value {
                    Some(x) => gen_emit_expr(f, x, ctx, &gen),
                    None => {
                        f.instruction(&Instruction::GlobalGet(G_NONE));
                    }
                }
                f.instruction(&Instruction::LocalSet(gen.scratch.b));
                gen_lget_gen(f);
                f.instruction(&Instruction::LocalGet(gen.scratch.b));
                sset(f, T_GENERATOR, F_GEN_VALUE);
                gen_lget_gen(f);
                f.instruction(&Instruction::I32Const(GS_DONE));
                sset(f, T_GENERATOR, F_GEN_STATE);
                f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
                f.instruction(&Instruction::Return);
            }
        }
        if in_wrap {
            // try_table End + null payload -> $cx End: exc|null
            f.instruction(&Instruction::End); // try_table
            f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
            f.instruction(&Instruction::End); // $cx
            f.instruction(&Instruction::LocalSet(gen.scratch.s_pad));
            // handler chain (scratch.s_pad = exc|null; null never reaches
            // here — the fall-through path br'd out of $cx already... it
            // DIDN'T: the body fall-through arrives here with NULL. Guard:
            f.instruction(&Instruction::Block(BlockType::Empty)); // $hdone
            {
                f.instruction(&Instruction::LocalGet(gen.scratch.s_pad));
                f.instruction(&Instruction::RefIsNull);
                f.instruction(&Instruction::If(BlockType::Empty));
                // null = body completed normally -> exit the wrapper (goto
                // after, extra = $hdone+If = 2... reuse goto_x at chain
                // level: labels open = $hdone(0), If(1), B.. , $done, Loop
                {
                    let wid = wrap_of[k].unwrap();
                    let w = &plan.wrappers[wid];
                    let after = w.after;
                    if after >= n_segs {
                        f.instruction(&Instruction::Br((n_segs - k + 2) as u32));
                    } else if after > k {
                        // B(after-1) sits at depth after-k from here
                        f.instruction(&Instruction::Br((after - k) as u32));
                    } else {
                        f.instruction(&Instruction::I32Const(after as i32));
                        f.instruction(&Instruction::LocalSet(2));
                        f.instruction(&Instruction::Br((n_segs - k + 3) as u32));
                    }
                }
                f.instruction(&Instruction::End);
                // handler match chain
                let wid = wrap_of[k].unwrap();
                let w = &plan.wrappers[wid];
                for (hi, h) in w.handlers.iter().enumerate() {
                    f.instruction(&Instruction::LocalGet(gen.scratch.s_pad));
                    match &h.ty {
                        None => {
                            f.instruction(&Instruction::I32Const(1));
                        }
                        Some(ty) => {
                            gen_emit_expr(f, ty, ctx, &gen);
                            f.instruction(&Instruction::Call(fx::FN_EXC_MATCH));
                        }
                    }
                    f.instruction(&Instruction::If(BlockType::Empty));
                    {
                        // bind `as n` non-park prefix handled in the
                        // HandlerPrefix segment; here only match routing:
                        // run the prefix stmts (bind-name when no park)
                        if h.resume.is_none() {
                            // no yield in the handler: prefix stmts are the
                            // whole body — emit inline then goto after.
                            // (prefix carries post via HandlerPlan? No:
                            // split_handler_body put everything in prefix
                            // when park_value is None.)
                            for s in &h.prefix {
                                gen_emit_stmt(f, s, ctx, &gen);
                            }
                            if let Some(n) = &h.name {
                                f.instruction(&Instruction::LocalGet(gen.scratch.s_pad));
                                gen_store_slot(f, &gen, n.as_str());
                            }
                            let after = w.after;
                            if after >= n_segs {
                                f.instruction(&Instruction::Br((n_segs - k + 2) as u32));
                            } else if after > k {
                                f.instruction(&Instruction::Br((after - k) as u32));
                            } else {
                                f.instruction(&Instruction::I32Const(after as i32));
                                f.instruction(&Instruction::LocalSet(2));
                                f.instruction(&Instruction::Br((n_segs - k + 3) as u32));
                            }
                        } else {
                            // park-yielding handler: dispatch to the
                            // staged HandlerPrefix segment (it carries the
                            // prefix stmts + park + resume wiring).
                            let pid = prefix_seg_of[k]
                                .expect("park-yield handler has a staged prefix seg");
                            f.instruction(&Instruction::I32Const(pid as i32));
                            f.instruction(&Instruction::LocalSet(2));
                            f.instruction(&Instruction::Br((n_segs - k + 3) as u32));
                        }
                    }
                    f.instruction(&Instruction::End); // If(hi)
                    let _ = hi;
                }
                // unmatched: rethrow
                f.instruction(&Instruction::LocalGet(gen.scratch.s_pad));
                f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
                f.instruction(&Instruction::Throw(TAG_PYEXC));
            }
            f.instruction(&Instruction::End); // $hdone
        }
    }
    f.instruction(&Instruction::End); // $sink End (last segment block's
    // End already closed B(n-1); falling here = done region entry, and the
    // br_table default (depth n from dispatch) lands HERE directly).
    // done region: $value = None, state = DONE, return null. The entry is
    // POP-INSENSITIVE: dispatch (state = n) arrives with an i32 on the
    // stack (the br_table operand, auto-popped by the br), a resume with a
    // pyobj — so DON'T Return (that would type-require a value); fall
    // through $done's End instead, which pops nothing. The step fn's
    // (result oref) contract is met by the FT filler after Loop's End.
    gen_lget_gen(f);
    f.instruction(&Instruction::GlobalGet(G_NONE));
    sset(f, T_GENERATOR, F_GEN_VALUE);
    gen_lget_gen(f);
    f.instruction(&Instruction::I32Const(GS_DONE));
    sset(f, T_GENERATOR, F_GEN_STATE);
    f.instruction(&Instruction::End); // $done End
    f.instruction(&Instruction::End); // Loop End
    f.instruction(&Instruction::RefNull(hc(T_PYOBJ))); // FT result filler
    f.instruction(&Instruction::End); // FUNCTION body End
}

/// Statement emit inside a generator segment (Seq bodies; no yields reach
/// here — the planner split them out).
fn gen_emit_stmt(f: &mut Function, stmt: &Stmt, ctx: &mut EmitCtx, gen: &GenCtx) {
    match stmt {
        Stmt::Line(_) => {} // no frame pokes in step fns
        Stmt::Pass => {}
        Stmt::Global(_) | Stmt::Nonlocal(_) => {}
        Stmt::ExprStmt(e) => gen_emit_expr_stmt(f, e, ctx, gen),
        Stmt::Assign(targets, value) => gen_emit_assign(f, targets, value, ctx, gen),
        Stmt::AugAssign(t, op, v) => {
            let ap_ir::item::Target::Name(n) = t else {
                unreachable!("scan vetted augassign targets");
            };
            f.instruction(&Instruction::I32Const(binop_op(*op)));
            gen_emit_name_load(f, n.as_str(), ctx, gen);
            gen_emit_expr(f, v, ctx, gen);
            f.instruction(&Instruction::Call(fx::FN_BINOP));
            gen_store_slot(f, gen, n.as_str());
        }
        Stmt::If(cond, then, els) => {
            gen_emit_cond(f, cond, ctx, gen);
            f.instruction(&Instruction::If(BlockType::Empty));
            for s in then {
                gen_emit_stmt(f, s, ctx, gen);
            }
            f.instruction(&Instruction::Else);
            for s in els {
                gen_emit_stmt(f, s, ctx, gen);
            }
            f.instruction(&Instruction::End);
        }
        Stmt::Raise(e, cause) => match e {
            None => {
                f.instruction(&Instruction::GlobalGet(ctx.fg.active_exc));
                f.instruction(&Instruction::Call(fx::FN_EXC_RAISE));
                f.instruction(&Instruction::Drop);
            }
            Some(expr) => {
                gen_emit_expr(f, expr, ctx, gen);
                if let Some(c) = cause {
                    f.instruction(&Instruction::GlobalGet(ctx.fg.active_exc));
                    gen_emit_expr(f, c, ctx, gen);
                    f.instruction(&Instruction::Call(fx::FN_EXC_CTX_SET));
                }
                f.instruction(&Instruction::Call(fx::FN_EXC_RAISE));
                f.instruction(&Instruction::Drop);
            }
        },
        other => unreachable!("gen seq stmt vetted (yields split out): {other:?}"),
    }
}

/// Assignment inside a generator segment (Name targets only).
fn gen_emit_assign(
    f: &mut Function,
    targets: &[ap_ir::item::Target],
    value: &Expr,
    ctx: &mut EmitCtx,
    gen: &GenCtx,
) {
    gen_emit_expr(f, value, ctx, gen);
    let ap_ir::item::Target::Name(n) = &targets[0] else {
        unreachable!("scan vetted gen assign targets");
    };
    gen_store_slot(f, gen, n.as_str());
}

/// Statement-level (value-discarding) expr inside a generator segment.
fn gen_emit_expr_stmt(f: &mut Function, e: &Expr, ctx: &mut EmitCtx, gen: &GenCtx) {
    // print keeps its statement shape (no Drop)
    let is_print = matches!(
        e,
        Expr::Call(func, _)
            if matches!(func.as_ref(), Expr::Name(n) if n.as_str() == "print")
                && !ctx.scan.fns.iter().any(|uf| uf.name == "print")
    );
    gen_emit_expr(f, e, ctx, gen);
    if !is_print {
        f.instruction(&Instruction::Drop);
    }
}

/// Extract a StopIteration payload (args[0] or None) into scratch.b — the
/// yield-from completion value (PEP 380: `r = yield from it` binds the
/// delegate's RETURN, which the driver parked as the SI value).
fn gen_extract_si_payload(f: &mut Function, gen: &GenCtx) {
    // exc = scratch.a
    f.instruction(&Instruction::LocalGet(gen.scratch.a));
    f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
    sget(f, T_EXCINST, F_EXC_ARGS);
    f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
    sget(f, T_TUPLE, F_TUPLE_ITEMS);
    f.instruction(&Instruction::ArrayLen);
    f.instruction(&Instruction::If(BlockType::Result(pyobj_ref())));
    // args non-empty: args[0]
    f.instruction(&Instruction::LocalGet(gen.scratch.a));
    f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
    sget(f, T_EXCINST, F_EXC_ARGS);
    f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
    sget(f, T_TUPLE, F_TUPLE_ITEMS);
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
    f.instruction(&Instruction::Else);
    f.instruction(&Instruction::GlobalGet(G_NONE));
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::LocalSet(gen.scratch.b));
}

/// PARK at a yield site: spill mirrors -> $gen.$locals, $value = yieldval
/// (on stack), $state = park, return yieldval.
fn gen_park_yield(f: &mut Function, gen: &GenCtx, park: i32) {
    // stash the yield value
    f.instruction(&Instruction::LocalSet(gen.scratch.b));
    // clear any staged throw-in ($exn was consumed by the resume that
    // reached this park; gen_close's GeneratorExit must not linger or the
    // next resume re-sees it).
    gen_lget_gen(f);
    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
    sset(f, T_GENERATOR, F_GEN_EXN);
    for i in 0..gen.slot_table.len() as u32 {
        gen_lget_gen(f);
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_GENERATOR,
            field_index: F_GEN_LOCALS,
        });
        f.instruction(&Instruction::I32Const(i as i32));
        f.instruction(&Instruction::LocalGet(gen.mirror_base + i));
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    gen_lget_gen(f);
    f.instruction(&Instruction::LocalGet(gen.scratch.b));
    sset(f, T_GENERATOR, F_GEN_VALUE);
    gen_lget_gen(f);
    f.instruction(&Instruction::I32Const(park));
    sset(f, T_GENERATOR, F_GEN_STATE);
    f.instruction(&Instruction::LocalGet(gen.scratch.b));
    f.instruction(&Instruction::Return);
}

/// Name load inside a generator state (slot table; free/global names fall
/// through to the standard resolution — the mirrors ONLY carry local slots).
fn gen_emit_name_load(f: &mut Function, name: &str, ctx: &mut EmitCtx, gen: &GenCtx) {
    if gen.slot_table.iter().any(|n| n == name) {
        gen_load_slot(f, gen, name);
        return;
    }
    emit_name_load(f, name, ctx);
}

/// Condition (i32 truthiness) inside a generator state.
fn gen_emit_cond(f: &mut Function, cond: &Expr, ctx: &mut EmitCtx, gen: &GenCtx) {
    if let Expr::Compare(ops, operands) = cond {
        for (i, op) in ops.iter().enumerate() {
            f.instruction(&Instruction::I32Const(cmp_op(*op)));
            gen_emit_expr(f, &operands[i], ctx, gen);
            gen_emit_expr(f, &operands[i + 1], ctx, gen);
            f.instruction(&Instruction::Call(fx::FN_COMPARE));
        }
        for _ in 1..ops.len() {
            f.instruction(&Instruction::I32And);
        }
        return;
    }
    gen_emit_expr(f, cond, ctx, gen);
    f.instruction(&Instruction::Call(fx::FN_TRUTHY));
}

/// Expression emit inside a generator state. Names route to the slots when
/// local; the rest delegates to emit_expr EXCEPT call arguments and nested
/// Name loads — those re-route through THIS fn via a rewriting walk:
/// gen-wrap exprs that CONTAIN names by cloning with name substitution is
/// overkill; instead we handle the shapes the corpus uses directly (the
/// rest falls to emit_expr, whose Name loads resolve via ctx.scope — valid
/// ONLY when the name is global/free, not a slot: acceptable because
/// emit_expr's Name arm would read a phantom local in the step frame... so
/// we WALK the expr tree and route ALL Name leaves through the slot table).
fn gen_emit_expr(f: &mut Function, e: &Expr, ctx: &mut EmitCtx, gen: &GenCtx) {
    match e {
        Expr::Name(n) => gen_emit_name_load(f, n.as_str(), ctx, gen),
        Expr::BinOp(op, a, b) => {
            f.instruction(&Instruction::I32Const(binop_op(*op)));
            gen_emit_expr(f, a, ctx, gen);
            gen_emit_expr(f, b, ctx, gen);
            f.instruction(&Instruction::Call(fx::FN_BINOP));
        }
        Expr::UnaryOp(op, a) => {
            let code_op = match op {
                UnaryOp::Plus => -1,
                UnaryOp::Minus => OPS::NEG,
                UnaryOp::Invert => OPS::INVERT,
                UnaryOp::Not => OPS::NOT,
            };
            if code_op < 0 {
                gen_emit_expr(f, a, ctx, gen);
            } else {
                f.instruction(&Instruction::I32Const(code_op));
                gen_emit_expr(f, a, ctx, gen);
                f.instruction(&Instruction::Call(fx::FN_UNARY));
            }
        }
        Expr::Compare(ops, operands) => {
            for (i, op) in ops.iter().enumerate() {
                f.instruction(&Instruction::I32Const(cmp_op(*op)));
                gen_emit_expr(f, &operands[i], ctx, gen);
                gen_emit_expr(f, &operands[i + 1], ctx, gen);
                f.instruction(&Instruction::Call(fx::FN_COMPARE));
            }
            for _ in 1..ops.len() {
                f.instruction(&Instruction::I32And);
            }
            f.instruction(&Instruction::LocalSet(gen.scratch.cmp));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("bool").expect("TYPE_ORDER carries bool"),
            ));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(gen.scratch.cmp));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
            f.instruction(&Instruction::StructNew(T_INT));
        }
        Expr::Call(func, args) => {
            // resume markers
            if let Expr::Name(n) = func.as_ref() {
                if n.as_str() == "__ap_yv" || n.as_str() == "__ap_yf" {
                    gen_lget_gen(f);
                    sget(f, T_GENERATOR, F_GEN_VALUE);
                    return;
                }
            }
            gen_emit_call(f, func, args, ctx, gen);
        }
        Expr::Attribute(obj, name) => {
            gen_emit_expr(f, obj, ctx, gen);
            push_interned_str(f, name.as_str(), ctx);
            f.instruction(&Instruction::Call(fx::FN_ATTR_GET));
        }
        Expr::LitStr(s) => push_interned_str(f, s, ctx),
        Expr::LitInt(v) => {
            f.instruction(&Instruction::I64Const(*v as i64));
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        }
        Expr::LitBigInt(digits) => {
            let v: i128 = digits.parse().unwrap_or(0);
            let wrapped = if v > i64::MAX as i128 || v < i64::MIN as i128 {
                (v & 0xFFFF_FFFF_FFFF_FFFF) as i64
            } else {
                v as i64
            };
            f.instruction(&Instruction::I64Const(wrapped));
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        }
        Expr::LitNone => {
            f.instruction(&Instruction::GlobalGet(G_NONE));
        }
        Expr::LitBool(b) => {
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I64Const(*b as i64));
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
            f.instruction(&Instruction::Call(fx::FN_BOOL_OBJ));
        }
        Expr::List(items) => {
            for it in items {
                gen_emit_expr(f, it, ctx, gen);
            }
            // $list.items is the MUT array type (see the plain-scope arm).
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR,
                array_size: items.len() as u32,
            });
            f.instruction(&Instruction::LocalSet(gen.scratch.objarr));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("list").unwrap_or(0),
            ));
            f.instruction(&Instruction::I32Const(items.len() as i32));
            f.instruction(&Instruction::I32Const(items.len() as i32));
            f.instruction(&Instruction::LocalGet(gen.scratch.objarr));
            f.instruction(&Instruction::StructNew(T_LIST));
        }
        Expr::Tuple(items) => {
            for it in items {
                gen_emit_expr(f, it, ctx, gen);
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: items.len() as u32,
            });
            f.instruction(&Instruction::LocalSet(gen.scratch.roarr));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("tuple").unwrap_or(0),
            ));
            f.instruction(&Instruction::LocalGet(gen.scratch.roarr));
            f.instruction(&Instruction::StructNew(T_TUPLE));
        }
        Expr::BoolOp(BoolOp::And, ops) => gen_emit_bool_op(f, ops, false, ctx, gen),
        Expr::BoolOp(BoolOp::Or, ops) => gen_emit_bool_op(f, ops, true, ctx, gen),
        Expr::IfExp(c, t, e2) => {
            gen_emit_expr(f, c, ctx, gen);
            f.instruction(&Instruction::Call(fx::FN_TRUTHY));
            f.instruction(&Instruction::If(BlockType::Empty));
            ctx.depth += 1;
            gen_emit_expr(f, t, ctx, gen);
            f.instruction(&Instruction::LocalSet(gen.scratch.a));
            ctx.depth -= 1;
            f.instruction(&Instruction::Else);
            ctx.depth += 1;
            gen_emit_expr(f, e2, ctx, gen);
            f.instruction(&Instruction::LocalSet(gen.scratch.a));
            ctx.depth -= 1;
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(gen.scratch.a));
        }
        other => emit_expr(f, other, ctx),
    }
}

fn gen_emit_bool_op(
    f: &mut Function,
    ops: &[Expr],
    is_or: bool,
    ctx: &mut EmitCtx,
    gen: &GenCtx,
) {
    let res = gen.scratch.a;
    let entry = ctx.depth;
    f.instruction(&Instruction::Block(BlockType::Empty));
    ctx.depth += 1;
    let last = ops.len() - 1;
    for (i, op) in ops.iter().enumerate() {
        gen_emit_expr(f, op, ctx, gen);
        if i < last {
            f.instruction(&Instruction::LocalSet(res));
            f.instruction(&Instruction::LocalGet(res));
            f.instruction(&Instruction::Call(fx::FN_TRUTHY));
            if is_or {
                f.instruction(&Instruction::I32Eqz);
            }
            f.instruction(&Instruction::If(BlockType::Empty));
            ctx.depth += 1;
            f.instruction(&Instruction::LocalGet(res));
            f.instruction(&Instruction::Drop);
            ctx.depth -= 1;
            f.instruction(&Instruction::Else);
            ctx.depth += 1;
            f.instruction(&Instruction::LocalGet(res));
            f.instruction(&Instruction::Br(ctx.depth - entry - 1));
            ctx.depth -= 1;
            f.instruction(&Instruction::End);
        } else {
            f.instruction(&Instruction::LocalSet(res));
        }
    }
    ctx.depth -= 1;
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::LocalGet(res));
}

/// Call emission inside a generator state (gen-scope arg evaluation).
fn gen_emit_call(
    f: &mut Function,
    func: &Expr,
    args: &[Arg],
    ctx: &mut EmitCtx,
    gen: &GenCtx,
) {
    if let Expr::Name(name) = func {
        // STATIC GENERATOR call: build a parked $generator.
        // Shadowing = a NON-global res entry (Local/OwnCell/FreeCell). A
        // module-level gen name rides the child's free list as Res::Global
        // (03.04f: contains_key flagged it shadowed, the static arm never
        // fired, and the generic dispatch call_indirect'ed FT_PY_RO against
        // the FT_GEN_STEP body — trap on the first delegate resume).
        let shadowed = matches!(
            ctx.scope.res.get(name.as_str()),
            Some(Res::Local(_)) | Some(Res::OwnCell(_)) | Some(Res::FreeCell(_))
        );
        let slots: Vec<usize> = ctx
            .scan
            .fns
            .iter()
            .enumerate()
            .filter(|(_, uf)| uf.name == name.as_str() && uf.is_generator)
            .map(|(i, _)| i)
            .collect();
        if slots.len() == 1
            && !shadowed
            && args.iter().all(|a| matches!(a, Arg::Pos(_)))
            && args.len() == ctx.scan.fns[slots[0]].params.len()
        {
            gen_emit_generator_new(f, slots[0], args, ctx, gen);
            return;
        }
        if !ctx.scan.fns.iter().any(|uf| uf.name == name.as_str())
            && is_supported_builtin(name.as_str())
        {
            gen_emit_builtin(f, name.as_str(), args, ctx, gen);
            return;
        }
        // exception/user CLASS call (raise ValueError('mid') inside a gen
        // body): call_of materializes the instance. NO $function cast — a
        // $type would trap it.
        {
            let is_class = !ctx.scan.module_binds.iter().any(|n| n == name.as_str())
                && crate::types::exc_type_global(name.as_str()).is_some();
            let is_user_class = ctx.scan.classes.iter().any(|c| c == name.as_str());
            if is_class || is_user_class {
                gen_emit_name_load(f, name.as_str(), ctx, gen);
                for a in args {
                    let Arg::Pos(x) = a else {
                        unreachable!("scan vetted positional args");
                    };
                    gen_emit_expr(f, x, ctx, gen);
                }
                f.instruction(&Instruction::ArrayNewFixed {
                    array_type_index: T_OBJ_ARR_RO,
                    array_size: args.len() as u32,
                });
                f.instruction(&Instruction::Call(fx::FN_CALL_OF));
                return;
            }
        }
        // static plain-fn call (exact arity): direct call with gen arg eval
        let fslots: Vec<usize> = ctx
            .scan
            .fns
            .iter()
            .enumerate()
            .filter(|(_, uf)| uf.name == name.as_str() && !uf.is_generator)
            .map(|(i, _)| i)
            .collect();
        if fslots.len() == 1
            && !shadowed
            && args.iter().all(|a| matches!(a, Arg::Pos(_)))
            && args.len() == ctx.scan.fns[fslots[0]].params.len()
            && args.len() <= 2
        {
            gen_emit_name_load(f, name.as_str(), ctx, gen);
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            let mut stashes: Vec<u32> = Vec::new();
            for a in args {
                let Arg::Pos(x) = a else { unreachable!() };
                gen_emit_expr(f, x, ctx, gen);
                let st = if stashes.is_empty() { gen.scratch.a } else { gen.scratch.b };
                stashes.push(st);
                f.instruction(&Instruction::LocalSet(st));
            }
            for st in &stashes {
                f.instruction(&Instruction::LocalGet(*st));
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: args.len() as u32,
            });
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::Call(fn_idx(fslots[0])));
            return;
        }
    }
    // generic dispatch
    emit_function_ref(f, func, ctx);
    for a in args {
        let Arg::Pos(x) = a else {
            unreachable!("scan vetted positional args");
        };
        gen_emit_expr(f, x, ctx, gen);
    }
    f.instruction(&Instruction::ArrayNewFixed {
        array_type_index: T_OBJ_ARR_RO,
        array_size: args.len() as u32,
    });
    f.instruction(&Instruction::RefNull(hc(T_DICT)));
    f.instruction(&Instruction::Call(fx::FN_DISPATCH));
}

fn gen_emit_builtin(
    f: &mut Function,
    name: &str,
    args: &[Arg],
    ctx: &mut EmitCtx,
    gen: &GenCtx,
) {
    let mut pos: Vec<&Expr> = Vec::new();
    for a in args {
        let Arg::Pos(x) = a else { unreachable!() };
        pos.push(x);
    }
    match name {
        "print" => {
            for (i, a) in pos.iter().enumerate() {
                if i > 0 {
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                    f.instruction(&Instruction::I32Const(b' ' as i32));
                    f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
                        offset: 0,
                        align: 0,
                        target: wasm_encoder::MemArgTarget::Memory(0),
                        ordering: None,
                    }));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                    f.instruction(&Instruction::I32Const(1));
                    f.instruction(&Instruction::I32Add);
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
                }
                gen_emit_expr(f, a, ctx, gen);
                f.instruction(&Instruction::Call(fx::FN_PY_STR));
                f.instruction(&Instruction::LocalSet(gen.scratch.s));
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                f.instruction(&Instruction::LocalGet(gen.scratch.s));
                f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
            }
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
            f.instruction(&Instruction::Call(fx::FN_PRINT_FLUSH));
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
        }
        "next" => {
            // 1-arg: value = iter_next(it) (SI propagates). 2-arg:
            // canonical Block($cx result exc) + try_table; retry pushes
            // ONLY the iterator (the default was stashed pre-block).
            let posn: Vec<&Expr> = args
                .iter()
                .filter_map(|a| match a {
                    Arg::Pos(x) => Some(x),
                    _ => None,
                })
                .collect();
            if posn.len() == 2 {
                if let [Arg::Pos(it), Arg::Pos(def)] = args {
                    emit_expr(f, def, ctx);
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_B));
                    emit_expr(f, it, ctx);
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::Block(BlockType::Result(
                        ValType::Ref(wasm_encoder::RefType {
                            nullable: true,
                            heap_type: hc(T_EXCEPTION),
                        }),
                    ))); // $cx
                    let catches = [wasm_encoder::Catch::One {
                        tag: TAG_PYEXC,
                        label: 0,
                    }];
                    f.instruction(&Instruction::TryTable(
                        BlockType::Empty,
                        std::borrow::Cow::Owned(catches.to_vec()),
                    ));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::End); // try_table
                    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION))); // payload
                    f.instruction(&Instruction::End); // $cx: exc|null
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_S));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_S));
                    f.instruction(&Instruction::RefIsNull);
                    f.instruction(&Instruction::If(BlockType::Result(pyobj_ref())));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::Else);
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_B));
                    f.instruction(&Instruction::End);
                } else {
                    unreachable!("scan vetted next() arg kinds");
                }
            } else {
                for a in args {
                    let Arg::Pos(x) = a else {
                        unreachable!("scan vetted positional args");
                    };
                    emit_expr(f, x, ctx);
                }
                f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
            }
        }
        "range" => {
            match pos.len() {
                1 => {
                    f.instruction(&Instruction::I64Const(0));
                    f.instruction(&Instruction::LocalSet(53));
                    gen_emit_expr(f, pos[0], ctx, gen);
                    f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
                    sget(f, T_INT, F_INT_SMI);
                    f.instruction(&Instruction::LocalSet(54));
                    f.instruction(&Instruction::I64Const(1));
                    f.instruction(&Instruction::LocalSet(55));
                }
                2 => {
                    gen_emit_expr_i64(f, pos[0], ctx, gen);
                    f.instruction(&Instruction::LocalSet(53));
                    gen_emit_expr_i64(f, pos[1], ctx, gen);
                    f.instruction(&Instruction::LocalSet(54));
                    f.instruction(&Instruction::I64Const(1));
                    f.instruction(&Instruction::LocalSet(55));
                }
                3 => {
                    gen_emit_expr_i64(f, pos[0], ctx, gen);
                    f.instruction(&Instruction::LocalSet(53));
                    gen_emit_expr_i64(f, pos[1], ctx, gen);
                    f.instruction(&Instruction::LocalSet(54));
                    gen_emit_expr_i64(f, pos[2], ctx, gen);
                    f.instruction(&Instruction::LocalSet(55));
                }
                _ => unreachable!("scan vetted range() arity"),
            }
            emit_range_tail(f);
        }
        "iter" => {
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
        }
        "list" => {
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
            f.instruction(&Instruction::Call(fx::FN_LIST_DRAIN));
        }
        "tuple" => {
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
            f.instruction(&Instruction::Call(fx::FN_LIST_DRAIN));
            // $list -> $tuple: snapshot via iter (list of the same items)
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
            f.instruction(&Instruction::Call(fx::FN_LIST_DRAIN));
            f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_LIST,
                field_index: F_LIST_ITEMS,
            });
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("tuple").unwrap_or(0),
            ));
            f.instruction(&Instruction::RefNull(hc(T_TYPE)));
            f.instruction(&Instruction::StructNew(T_PYOBJ));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("tuple").unwrap_or(0),
            ));
            f.instruction(&Instruction::RefNull(hc(T_TYPE)));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            unreachable!("tuple() inside a generator body is unreachable on the corpus slice");
        }
        "sum" => {
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
            f.instruction(&Instruction::Call(fx::FN_SUM_DRAIN));
        }
        "len" => {
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::Call(fx::FN_LEN));
        }
        "str" | "repr" => {
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::Call(fx::FN_PY_STR));
        }
        "bool" => {
            f.instruction(&Instruction::I32Const(0));
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::Call(fx::FN_BOOL_OBJ));
        }
        "isinstance" => {
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::Call(fx::FN_ISINST));
            gen_bool_object(f, gen);
        }
        "issubclass" => {
            for a in &pos {
                gen_emit_expr(f, a, ctx, gen);
            }
            f.instruction(&Instruction::Call(fx::FN_ISSUB));
            gen_bool_object(f, gen);
        }
        _ => unreachable!("gen builtin vetted: {name}"),
    }
}

fn gen_bool_object(f: &mut Function, gen: &GenCtx) {
    f.instruction(&Instruction::LocalSet(gen.scratch.cmp));
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("bool").expect("TYPE_ORDER carries bool"),
    ));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalGet(gen.scratch.cmp));
    f.instruction(&Instruction::I64ExtendI32U);
    f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
    f.instruction(&Instruction::StructNew(T_INT));
}

fn gen_emit_expr_i64(f: &mut Function, e: &Expr, ctx: &mut EmitCtx, gen: &GenCtx) {
    gen_emit_expr(f, e, ctx, gen);
    f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
    f.instruction(&Instruction::StructGet {
        struct_type_index: T_INT,
        field_index: F_INT_SMI,
    });
}

/// Build a parked $generator (call to generator slot) from a generator
/// state: locals array = [param values..., None pad], $function object,
/// $generator struct.
fn gen_emit_generator_new(
    f: &mut Function,
    slot: usize,
    args: &[Arg],
    ctx: &mut EmitCtx,
    gen: &GenCtx,
) {
    let uf = ctx.scan.fns[slot].clone();
    let nparams = uf.params.len() as u32;
    let n_slots = gen_slot_count(&uf);
    // $locals array -> scratch.objarr
    f.instruction(&Instruction::I32Const(n_slots as i32));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(gen.scratch.objarr));
    for (i, p) in uf.params.iter().enumerate() {
        f.instruction(&Instruction::LocalGet(gen.scratch.objarr));
        f.instruction(&Instruction::I32Const(i as i32));
        let Arg::Pos(x) = &args[i] else { unreachable!() };
        gen_emit_expr(f, x, ctx, gen);
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    for i in nparams..n_slots {
        f.instruction(&Instruction::LocalGet(gen.scratch.objarr));
        f.instruction(&Instruction::I32Const(i as i32));
        f.instruction(&Instruction::GlobalGet(G_NONE));
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    // $function object: [type, name, nparams, flags, param_names, func_idx]
    // -> $code; defaults(empty)/kwdefaults/cells/globals -> $function.
    // param_names -> scratch.objarr is BUSY holding locals; $code.param_names
    // is a MUT $obj_arr (T_OBJ_ARR), so the RO array.new_fixed shape traps
    // validation (03.04f). Build the MUT array into scratch.objarr2 (free
    // expressions), then LocalGet it into the StructNew field.
    f.instruction(&Instruction::I32Const(nparams as i32));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(gen.scratch.objarr2));
    for (i, p) in uf.params.iter().enumerate() {
        f.instruction(&Instruction::LocalGet(gen.scratch.objarr2));
        f.instruction(&Instruction::I32Const(i as i32));
        push_interned_str(f, p.name.as_str(), ctx);
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    // $generator field0 ($type) must sit UNDER the $function build on the
    // stack — push it before the $code build (03.04f: pushed after, so
    // struct.new 31 popped fn-as-type → validate/trap).
    f.instruction(&Instruction::RefNull(hc(T_TYPE)));
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("function").unwrap_or(0),
    ));
    push_interned_str(f, uf.name.as_str(), ctx);
    f.instruction(&Instruction::I32Const(nparams as i32));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalGet(gen.scratch.objarr2));
    f.instruction(&Instruction::I32Const(fn_idx(slot) as i32));
    f.instruction(&Instruction::StructNew(T_CODE));
    // defaults: empty (corpus slice); a defaulted gen fn would need the
    // def-time eval here — scan keeps gen defaults out of the slice.
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::RefNull(hc(T_DICT)));
    f.instruction(&Instruction::RefNull(hc(T_CELL_ARR)));
    f.instruction(&Instruction::RefNull(hc(T_DICT)));
    f.instruction(&Instruction::StructNew(T_FUNCTION));
    // $generator struct: [type(under fn), fn, state=0, locals(objarr), null,
    // null] — type was pushed before the $function build.
    f.instruction(&Instruction::I32Const(GS_START));
    f.instruction(&Instruction::LocalGet(gen.scratch.objarr));
    f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
    f.instruction(&Instruction::StructNew(T_GENERATOR));
}

fn gen_slot_count(uf: &UserFn) -> u32 {
    let mut t: Vec<String> = uf.params.iter().map(|p| p.name.to_string()).collect();
    for n in collect_local_binds(&uf.body) {
        if uf.params.iter().any(|p| p.name == n)
            || uf.own_cells.contains(&n)
            || uf.free.contains(&n)
            || uf.free_cells.contains(&n)
        {
            continue;
        }
        if !t.contains(&n) {
            t.push(n);
        }
    }
    // hidden suspension-surviving slots (for-it / yf-it): appended by BOTH
    // this count (via gen_hidden_slots) and the step fn's slot_table build —
    // the two MUST stay in lockstep (the mirror reload indexes them).
    let mut hidden = gen_hidden_slots(&uf.body);
    t.append(&mut hidden);
    t.len() as u32
}

/// Emit one user function under the uniform convention:
/// params: 0 = $function (free-cell source), 1 = args RO array, 2 = kwargs
/// (ignored on this slice); result: pyobj.
fn emit_user_fn(
    uf: &UserFn,
    slot: usize,
    scan: &Scan,
    res: &std::collections::HashMap<String, Res>,
    tab: &StrTab,
    g: &crate::prelude_classes::FrameGlobals,
    eh_wrap_top: bool,
) -> Function {
    let nparams = uf.params.len() as u32;
    // local layout: 0/1/2 = FT_PY_RO convention (fn, args, kwargs), then
    // 3..3+n = Python params, then PLAIN locals, then own cells, then free
    // cells, then the shared temp window (indices land on 42..49 by
    // construction: the plain-local count is body-dependent, so the temps
    // are addressed RELATIVE to cell_base + n_own + n_free below).
    let mut order: Vec<(u32, ValType)> = Vec::new();
    // NOTE: locals 0/1/2 are the FT params (declared by the function TYPE,
    // not by Function::new — wasm params precede locals; re-declaring them
    // here shifted every index by 3).
    let mut plain_idx = 3 + nparams;
    if !uf.is_generator {
        // FT_PY_RO convention: 0 = $function, 1 = args RO array, 2 = kwargs.
        // Python params live at locals 3..3+n, plain locals follow.
        for _ in 0..nparams {
            order.push((1, pyobj_ref()));
        }
        for n in collect_local_binds(&uf.body) {
            if uf.params.iter().any(|p| p.name == n)
                || uf.own_cells.contains(&n)
                || uf.free.contains(&n)
                || uf.free_cells.contains(&n)
            {
                continue;
            }
            order.push((1, pyobj_ref()));
            plain_idx += 1;
        }
    } else {
        // 03.04d generator step fn: 2 = state (i32), 3 = payload (oref),
        // 4..4+S = slot mirrors (oref). plain_idx lands PAST the gen window
        // so the pad below computes the shared temps correctly.
        plain_idx = 2 + gen_slot_count(uf) + 10;

        // 03.04d generator step fn: 2 = state (i32), 3 = payload (oref),
        // 4..4+S = slot mirrors (oref), then the 8-scratch gen window
        // [A, B, OBJARR, S, I64, CMP, ROARR, FN] — indices MUST match
        // GenScratch::for_slots.
        order.push((1, ValType::I32)); // 2 = state
        order.push((1, pyobj_ref())); // 3 = payload stash
        let n_slots = gen_slot_count(uf);
        for _ in 0..n_slots {
            order.push((1, pyobj_ref())); // 4..4+S mirrors
        }
        // GenScratch::for_slots layout: a(4+n) b(5+n) objarr(6+n) s(7+n)
        // [oref], i64v(8+n), cmp(9+n) [i32], roarr(10+n) [oref].
        order.push((1, pyobj_ref())); // a
        order.push((1, pyobj_ref())); // b
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR),
            }),
        )); // objarr
        order.push((1, pyobj_ref())); // s
        order.push((1, ValType::I64)); // i64v
        order.push((1, ValType::I32)); // cmp
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR_RO),
            }),
        )); // roarr
        order.push((1, pyobj_ref())); // s_pad (payload scratch)
        order.push((1, obj_arr_ref())); // objarr2 (MUT temp, 03.04f)
    }
    let cell_base = plain_idx;
    let n_own = uf.own_cells.len() as u32;
    let n_free = uf.free_cells.len() as u32;
    if !uf.is_generator {
        for _ in 0..n_own + n_free {
            order.push((
                1,
                ValType::Ref(RefType {
                    nullable: true,
                    heap_type: wasm_encoder::HeapType::Concrete(T_CELL),
                }),
            ));
        }
    }
    // temps: pad to the shared ABSOLUTE window (40..51) so every frame —
    // _start and user fns — addresses temps identically (TEMP_LOCAL=40,
    // TEMP_LOCAL_A=42 etc. are module constants). Filler types mirror
    // _start's: 40=i32, 41=i64, 42/43/45=oref, 44=mut obj_arr, 46=i64,
    // 47=i32, 48=RO arr, 49=$function, 50=obj_arr2, 51=cell_arr.
    {
        let used = if uf.is_generator {
            // params(0,1) + state + payload + slot mirrors + the 9-scratch
            // gen window — everything pushed BEFORE the pad (locals 2..13+n;
            // 03.04f added objarr2 at 12+n)
            13 + gen_slot_count(uf)
        } else {
            3 + nparams + (plain_idx - (3 + nparams)) + n_own + n_free
        };
        let pad = 40u32.saturating_sub(used);
        for _ in 0..pad {
            order.push((1, pyobj_ref()));
        }
        order.push((1, ValType::I32)); // 40
        order.push((1, ValType::I64)); // 41
        order.push((1, pyobj_ref())); // 42
        order.push((1, pyobj_ref())); // 43
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR),
            }),
        )); // 44
        order.push((1, pyobj_ref())); // 45
        order.push((1, ValType::I64)); // 46
        order.push((1, ValType::I32)); // 47
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR_RO),
            }),
        )); // 48
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_FUNCTION),
            }),
        )); // 49
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR),
            }),
        )); // 50
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_CELL_ARR),
            }),
        )); // 51
        order.push((1, pyobj_ref())); // 52: iterator stash (03.04c For/gen)
        // 03.04c range() scratch: 53/54/55 = i64 (start/stop/step),
        // 56 = i32 count, 57 = mut $obj_arr snapshot, 58 = pad, 59 = i32 i.
        order.push((3, ValType::I64)); // 53..55
        order.push((1, ValType::I32)); // 56
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_OBJ_ARR),
            }),
        )); // 57
        order.push((1, ValType::I32)); // 58
        order.push((1, ValType::I32)); // 59
    }

    let mut f = Function::new(order);
    // prologue: allocate OWN cells fresh per call (recursion reentrancy)
    for i in 0..n_own {
        f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_PYOBJ,
        )));
        f.instruction(&Instruction::StructNew(T_CELL));
        f.instruction(&Instruction::LocalSet(cell_base + i));
    }
    // 03.03 traceback frame push (leaf scope 6): every user fn frame
    // enters the linked $tb_frame stack; the throw path skips epilogues,
    // so the chain at throw time = exactly the traceback frames.
    // (03.04d: skipped ENTIRELY for generator step fns — no frame owned.)
    if !uf.is_generator {
        f.instruction(&Instruction::GlobalGet(
            builtin_type_global("str").unwrap_or(0),
        ));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Const(uf.name.len() as i32));
        // the fn name is scan-interned (post-scan intern pass); compute
        // its passive-segment offset like push_interned_str (the base is
        // the combined name blob)
        let idx = scan
            .strings
            .iter()
            .position(|b| b.as_slice() == uf.name.as_bytes())
            .expect("scan interned the fn name");
        let mut n_off = TYPE_ORDER
            .iter()
            .chain(EXC_TREE.iter().map(|(n, _)| n))
            .map(|n| n.len() as u32)
            .sum::<u32>();
        for b in scan.strings.iter().take(idx) {
            n_off += b.len() as u32;
        }
        let n_len = uf.name.len() as i32;
        f.instruction(&Instruction::I32Const(n_off as i32));
        f.instruction(&Instruction::I32Const(n_len));
        f.instruction(&Instruction::ArrayNewData {
            array_type_index: T_BYTE_ARR,
            array_data_index: 1,
        });
        f.instruction(&Instruction::StructNew(T_STR));
        // 03.04d gate: step fns must NOT frame_push AND must not even BUILD
        // the name $str (the build is unconditional in this block — gating
        // only the call left the $str dangling on the step fn's stack).
        if !uf.is_generator && eh_wrap_top {
            f.instruction(&Instruction::Call(fx::FN_FRAME_PUSH));
        }
    }
    // load free cells from the incoming $function's cells array
    // (03.04d: SKIPPED for generator step fns — gen prologue = emit_gen_step_fn)
    if n_free > 0 && !uf.is_generator {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_FUNCTION,
            field_index: F_FN_CELLS,
        });
        f.instruction(&Instruction::LocalSet(51)); // $cell_arr stash
        for i in 0..n_free {
            f.instruction(&Instruction::LocalGet(51));
            f.instruction(&Instruction::I32Const(i as i32));
            f.instruction(&Instruction::ArrayGet(T_CELL_ARR));
            f.instruction(&Instruction::LocalSet(cell_base + n_own + i));
        }
    }
    // bind Python params: args[i] when provided, else the def-time default
    // (S-02-7/8: defaults extend the accepted arity downward). argc is
    // args.len; defaults live on the $function (param 0) -> defaults[i].
    // Locals: 5=argc scratch, 6=default temp (the shared i64/oref temp
    // window is at 42+; 5/6 are reserved i32/i64 in every frame by the pad
    // — no, the pad reaches 42 exactly, so use the temp locals 42/43.)
    for (i, p) in uf.params.iter().enumerate() {
        // (03.04d: the FT_PY_RO param-bind prologue is SKIPPED for generator
        // step fns — their params arrive via the $generator.$locals array.)
        if uf.is_generator {
            break;
        }
        // if argc > i: local(3+i) = args[i]; else = defaults[d] where d is
        // the param's compact default_index (S-02-7: defaults only for
        // params WITH defaults). A default-less out-of-range index cannot
        // reach here: the dispatcher's arity window gates dynamic calls;
        // static calls are exact-arity by construction.
        let has_default = p.default_index.is_some();
        if has_default {
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32Const(i as i32));
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::If(BlockType::Empty));
        }
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(i as i32));
        f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
        f.instruction(&Instruction::LocalSet(3 + i as u32));
        if has_default {
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_FUNCTION,
                field_index: F_FN_DEFAULTS,
            });
            f.instruction(&Instruction::I32Const(
                p.default_index.unwrap() as i32,
            ));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
            f.instruction(&Instruction::LocalSet(3 + i as u32));
            f.instruction(&Instruction::End);
        }
    }
    // bind cell-backed params into their cells
    for (pi, p) in uf.params.iter().enumerate() {
        if let Some(ci) = uf.own_cells.iter().position(|n| n == p.name.as_str()) {
            f.instruction(&Instruction::LocalGet(cell_base + ci as u32));
            f.instruction(&Instruction::LocalGet(3 + pi as u32));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_CELL,
                field_index: F_CELL_V,
            });
        }
    }
    // scope for the body
    let scope = Scope {
        kind: ScopeKind::Fn,
        res: res.clone(),
        cell_base,
        n_own,
        params: uf.params.len(),
    };
    let mut buf_end = fx::PRINT_BUF;
    let mut ctx = EmitCtx {
        scan,
        res_maps: &[],
        tab,
        strs: &scan.strings,
        buf_ptr: &mut buf_end,
        scope: &scope,
        depth: 0,
        br_continue: None,
        br_break: None,
        cls_obj: None,
        fg: *g,
        // frame pokes/pops inside user fns only run when the caller's
        // module body is wrapped (03.03b gate)
        eh_wrap_top: eh_wrap_top,
    };
    if uf.is_generator {
        // emit_gen_step_fn closes $out, pushes the FT filler, and emits the
        // FUNCTION-body End itself.
        emit_gen_step_fn(&mut f, uf, &mut ctx);
    } else {
        for stmt in &uf.body {
            emit_stmt(&mut f, stmt, &mut ctx);
        }
    }
    // implicit `return None` (S-02-11) + the function-body End
    if uf.is_generator {
        // step fns: emit_gen_step_fn returned; nothing further.
    } else {
        f.instruction(&Instruction::GlobalGet(G_NONE));
        if eh_wrap_top {
            f.instruction(&Instruction::Call(fx::FN_FRAME_POP));
        }
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
    }
    f
}

/// 03.04d: build a parked $generator for a call to generator slot `slot`
/// (call-site scope: args via emit_expr, stashes ride the shared temp
/// window; NO defaults — scan keeps gen defaults off the slice).
fn emit_generator_call(f: &mut Function, slot: usize, args: &[Arg], ctx: &mut EmitCtx) {
    let uf = ctx.scan.fns[slot].clone();
    let nparams = uf.params.len() as u32;
    let n_slots = gen_slot_count(&uf);
    // 1. param_names -> TEMP_LOCAL_OBJARR2 (consumed by $code below)
    f.instruction(&Instruction::I32Const(nparams as i32));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR2));
    for (i, p) in uf.params.iter().enumerate() {
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR2));
        f.instruction(&Instruction::I32Const(i as i32));
        push_interned_str(f, p.name.as_str(), ctx);
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    // 1b. $locals snapshot -> TEMP_LOCAL_OBJARR (params + None pad) FIRST:
    // the $function build below leaves its result on the operand stack and
    // the $generator StructNew consumes it in field order (type, fn, ...).
    f.instruction(&Instruction::I32Const(n_slots as i32));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR));
    for (i, p) in uf.params.iter().enumerate() {
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR));
        f.instruction(&Instruction::I32Const(i as i32));
        let Arg::Pos(x) = &args[i] else { unreachable!() };
        emit_expr(f, x, ctx);
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    for i in nparams..n_slots {
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR));
        f.instruction(&Instruction::I32Const(i as i32));
        f.instruction(&Instruction::GlobalGet(G_NONE));
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    // 2. $function object: type field FIRST, then $code fields in order
    //    [name, nparams, flags, param_names, func_idx], then the $function
    //    tail [defaults, kwdefaults, cells, globals]. Result -> TEMP_LOCAL_A.
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("function").unwrap_or(0),
    ));
    push_interned_str(f, uf.name.as_str(), ctx);
    f.instruction(&Instruction::I32Const(nparams as i32));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR2));
    f.instruction(&Instruction::I32Const(fn_idx(slot) as i32));
    f.instruction(&Instruction::StructNew(T_CODE));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR)); // defaults
    f.instruction(&Instruction::RefNull(hc(T_DICT))); // kwdefaults
    f.instruction(&Instruction::RefNull(hc(T_CELL_ARR))); // cells
    f.instruction(&Instruction::RefNull(hc(T_DICT))); // globals
    f.instruction(&Instruction::StructNew(T_FUNCTION));
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_FN)); // stash the fn ($function-typed local)
    // 3. $generator struct: [null-type, fn, state=0, locals, null, null]
    f.instruction(&Instruction::RefNull(hc(T_TYPE)));
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_FN));
    f.instruction(&Instruction::I32Const(GS_START));
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR));
    f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
    f.instruction(&Instruction::StructNew(T_GENERATOR));
}

/// Build a `$function` object for scan slot `slot` on the stack, in the
/// CURRENT scope (`ctx`): defaults evaluate now (S-02-7); the cells array
/// carries the enclosing frame's cell refs for the child's `free_cells`
/// (S-02-13 shared cells — late binding falls out, S-02-12).
fn emit_function_object(f: &mut Function, slot: usize, ctx: &mut EmitCtx) {
    let uf = ctx.scan.fns[slot].clone();
    let nparams = uf.params.len() as u32;
    // 1. defaults (def-time evaluation, in order) -> MUT $obj_arr (44;
    // $function.defaults is (ref $obj_arr), non-null, so array.new_fixed's
    // RO output is unusable — build with new_default + set).
    f.instruction(&Instruction::I32Const(uf.defaults.len() as i32));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR));
    for (i, d) in uf.defaults.iter().enumerate() {
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR));
        f.instruction(&Instruction::I32Const(i as i32));
        emit_expr(f, d, ctx);
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    // 2. param names $str array -> mut obj arr (OBJARR2; 44 holds defaults)
    f.instruction(&Instruction::I32Const(nparams as i32));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR2));
    for (i, p) in uf.params.iter().enumerate() {
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR2));
        f.instruction(&Instruction::I32Const(i as i32));
        push_interned_str(f, p.name.as_str(), ctx);
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    // 3. $function type ref FIRST (field 0 of $function), then the $code
    // field sequence — StructNew(T_CODE) pops its own five fields off the
    // top, leaving [type, code] for the $function build.
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("function").unwrap_or(0),
    ));
    push_interned_str(f, uf.name.as_str(), ctx);
    f.instruction(&Instruction::I32Const(nparams as i32));
    f.instruction(&Instruction::I32Const(0)); // flags: plain positional
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR2)); // param_names
    f.instruction(&Instruction::I32Const(fn_idx(slot) as i32));
    f.instruction(&Instruction::StructNew(T_CODE));
    // 4. $function fields in order: type, code, defaults, kwdefaults, cells,
    //    globals. type + code just went on; push the rest.
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR)); // defaults
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_DICT,
    ))); // kwdefaults
    // cells: one cell ref per child free_cell, resolved in THIS scope
    let n_cells = uf.free_cells.len() as u32;
    if n_cells > 0 {
        for name in &uf.free_cells {
            push_resolved_cell(f, name, ctx, slot);
        }
        f.instruction(&Instruction::ArrayNewFixed {
            array_type_index: T_CELL_ARR,
            array_size: n_cells,
        });
    } else {
        f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_CELL_ARR,
        )));
    }
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_DICT,
    ))); // globals stub (module reads go direct)
    f.instruction(&Instruction::StructNew(T_FUNCTION));
}

/// Push the cell ref for `name` per the CURRENT scope (Fn scopes only).
fn push_resolved_cell(f: &mut Function, name: &str, ctx: &EmitCtx, _slot: usize) {
    match ctx.scope.res.get(name) {
        Some(Res::OwnCell(i)) => {
            f.instruction(&Instruction::LocalGet(ctx.scope.cell_base + i));
        }
        Some(Res::FreeCell(i)) => {
            f.instruction(&Instruction::LocalGet(
                ctx.scope.cell_base + ctx.scope.n_own + i,
            ));
        }
        _ => unreachable!("free cell must resolve to a cell in the enclosing scope"),
    }
}

/// Push an interned $str literal (scan-interned: literals + fn/param names).
fn push_interned_str(f: &mut Function, s: &str, ctx: &EmitCtx) {
    let idx = ctx
        .strs
        .iter()
        .position(|b| b.as_slice() == s.as_bytes())
        .unwrap_or_else(|| panic!("scan interned all strings: missing {s:?}"));
    // passive-segment base = the combined name blob (TYPE_ORDER + the
    // EXC_TREE names, leaf 03.03)
    let mut off = TYPE_ORDER
        .iter()
        .chain(EXC_TREE.iter().map(|(n, _)| n))
        .map(|n| n.len() as u32)
        .sum::<u32>();
    for b in ctx.strs.iter().take(idx) {
        off += b.len() as u32;
    }
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("str").unwrap_or(0),
    ));
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(s.len() as i32));
    f.instruction(&Instruction::I32Const(off as i32));
    f.instruction(&Instruction::I32Const(s.len() as i32));
    f.instruction(&Instruction::ArrayNewData {
        array_type_index: T_BYTE_ARR,
        array_data_index: 1,
    });
    f.instruction(&Instruction::StructNew(T_STR));
}

// ===========================================================================
// Statement emission
// ===========================================================================

/// Emit one statement.
fn emit_stmt(f: &mut Function, stmt: &Stmt, ctx: &mut EmitCtx) {
    match stmt {
        Stmt::Pass => {}
        // 03.03: source-line poke FIRST (traceback linenos, scope 6)
        Stmt::Line(line) => {
            // 03.03b: pokes only under the EH wrap (frame chain active);
            // without the wrap head is null and the poke is a no-op anyway.
            if ctx.eh_wrap_top {
                f.instruction(&Instruction::I32Const(*line as i32));
                f.instruction(&Instruction::Call(fx::FN_FRAME_LINE));
            }
        }
        Stmt::ExprStmt(e) => {
            // print is statement-shaped (flush consumes the buffer state,
            // leaving NO value): skip the Drop for it — everything else
            // leaves a pyobj to discard.
            if !matches!(
                e,
                Expr::Call(func, _)
                    if matches!(func.as_ref(), Expr::Name(n) if n.as_str() == "print")
                        && !ctx.scan.fns.iter().any(|uf| uf.name == "print")
            ) {
                emit_expr(f, e, ctx);
                f.instruction(&Instruction::Drop);
            } else {
                emit_expr(f, e, ctx);
            }
        }
        Stmt::Assign(targets, value) => {
            emit_expr(f, value, ctx);
            let t = &targets[0];
            // 03.03c: class-body plain assigns bind into the class namespace
            // dict (k = 42 under `class C:`) — attr_set(cls, name, v), same
            // shape as method defs. (Previously they leaked into the module
            // namespace, so instance/class reads of the name returned null.)
            if let (Some(cls), ap_ir::item::Target::Name(n)) = (ctx.cls_obj, t) {
                // stash via TEMP_LOCAL_B: cls itself rides TEMP_LOCAL_A inside
                // the class body (emit_class_def's stash) — clobbering A here
                // would pass the VALUE as attr_set's obj operand.
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_B));
                f.instruction(&Instruction::LocalGet(cls));
                push_interned_str(f, n.as_str(), ctx);
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_B));
                f.instruction(&Instruction::Call(fx::FN_ATTR_SET));
            } else if let ap_ir::item::Target::Attribute(obj, name) = t {
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                emit_expr(f, obj, ctx);
                push_interned_str(f, name.as_str(), ctx);
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                f.instruction(&Instruction::Call(fx::FN_ATTR_SET));
            } else {
                let ap_ir::item::Target::Name(n) = t else {
                    unreachable!("scan vetted assign targets");
                };
                emit_name_store(f, n.as_str(), ctx);
            }
        }
        Stmt::AugAssign(t, op, v) => {
            let ap_ir::item::Target::Name(n) = t else {
                unreachable!("scan vetted augassign targets");
            };
            f.instruction(&Instruction::I32Const(binop_op(*op)));
            emit_name_load(f, n.as_str(), ctx);
            emit_expr(f, v, ctx);
            f.instruction(&Instruction::Call(fx::FN_BINOP));
            emit_name_store(f, n.as_str(), ctx);
        }
        Stmt::Return(Some(e)) => {
            emit_expr(f, e, ctx);
            if ctx.eh_wrap_top {
                f.instruction(&Instruction::Call(fx::FN_FRAME_POP));
            }
            f.instruction(&Instruction::Return);
        }
        Stmt::Return(None) => {
            f.instruction(&Instruction::GlobalGet(G_NONE));
            if ctx.eh_wrap_top {
                f.instruction(&Instruction::Call(fx::FN_FRAME_POP));
            }
            f.instruction(&Instruction::Return);
        }
        Stmt::If(cond, then, els) => {
            emit_cond_i32(f, cond, ctx);
            f.instruction(&Instruction::If(BlockType::Empty));
            ctx.depth += 1;
            for s in then {
                emit_stmt(f, s, ctx);
            }
            ctx.depth -= 1;
            f.instruction(&Instruction::Else);
            ctx.depth += 1;
            for s in els {
                emit_stmt(f, s, ctx);
            }
            ctx.depth -= 1;
            f.instruction(&Instruction::End);
        }
        Stmt::While(cond, body, els) => {
            // While lowering (S-02-16 pre-test, S-02-18 continue/break).
            //
            // EMITTER NOTE (03.02c debugging): the textbook shape —
            //   Block(exit){ Loop(head){ cond; Eqz; BrIf; body; Br(0) } }
            // WEDGED under wasmtime 47 whenever the head region allocated
            // GC objects (compare's operand ints + the bool result build):
            // re-executing an allocating loop-head wedged the engine (the
            // identical sequence straight-line and one-shot works; zero-
            // iteration loops work; iterative loops hang). Restructured as
            // an If-wrapped bottom-test (do-while with a pre-entry test —
            // semantics preserved):
            //   cond; TRUTHY; If {          <- entry test, outside any loop
            //     Loop {
            //       Block { body }          <- continue: Br(0) = fall to
            //       cond; BrIf(0)           <- bottom test: truthy repeats
            //     }
            //   }
            //   else-body                   <- runs on normal exit (S-02-16)
            // break jumps past the If (skips else, S-02-18); the first
            // cond evaluation and every re-test evaluate the SAME expr.
            // $exit wraps Loop AND else: break targets its end (skips
            // else); normal cond-false exit falls into the else (S-02-16).
            f.instruction(&Instruction::Block(BlockType::Empty));
            ctx.depth += 1; // $exit label (break target; depth recorded below)
            emit_cond_i32(f, cond, ctx);
            f.instruction(&Instruction::If(BlockType::Empty));
            ctx.depth += 1;
            let saved_c = ctx.br_continue;
            let saved_b = ctx.br_break;
            f.instruction(&Instruction::Loop(BlockType::Empty));
            ctx.depth += 1;
            f.instruction(&Instruction::Block(BlockType::Empty));
            ctx.depth += 1;
            // label depths: at body-emit time ctx.depth = e+4 — body
            // Block label = e+4, Loop = e+3, entry If = e+2, $exit = e+1.
            // The Br arithmetic at a site is site-depth minus the label's
            // depth.
            ctx.br_continue = Some(ctx.depth); // body-Block end -> bottom test
            ctx.br_break = Some(ctx.depth - 3); // $exit end -> skips else
            for s in body {
                emit_stmt(f, s, ctx);
            }
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // body Block
            emit_cond_i32(f, cond, ctx);
            f.instruction(&Instruction::BrIf(0)); // truthy -> Loop head
            ctx.br_continue = saved_c;
            ctx.br_break = saved_b;
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // Loop
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // entry If
            for s in els {
                emit_stmt(f, s, ctx); // normal-exit only (S-02-16)
            }
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // $exit
        }
        Stmt::Break => {
            let target = ctx.br_break.expect("scan tracked loop nesting");
            f.instruction(&Instruction::Br(ctx.depth - target));
        }
        Stmt::Continue => {
            let target = ctx.br_continue.expect("scan tracked loop nesting");
            f.instruction(&Instruction::Br(ctx.depth - target));
        }
        // leaf 03.04c: For over an iterable (S-02-17): iter() once, then
        // next() until StopIteration; target binds in the current scope and
        // LEAKS after the loop; else on NORMAL exhaustion only.
        //
        // The catch region wraps ONLY the next() call (not the body): a
        // raise in the BODY propagates untouched (CPython: no else, no
        // swallow), and exhaustion is distinguished from body raises by an
        // exc_match against the StopIteration $type INSIDE the loop.
        //   Block $exit {                        <- break target (skips else)
        //     Block $brk {                       <- exhaustion target (falls to else)
        //       it = iter_of(e) [$it]
        //       Loop $head {
        //         Block $cx (result exc) {
        //           try_table [catch TAG -> $cx]
        //             x = iter_next(it); bind target   <- the ONLY guarded ops
        //           RefNull(exc)                 <- no-exception payload
        //         end ($cx)
        //         A = payload
        //         A null? -> Br 1 (Loop: next iteration)
        //         eq(A.type mro, StopIteration)? -> Br 2 ($brk: exhaustion, else)
        //         else: rethrow A (body-raise semantics never reach here;
        //              this arm catches iterator-side raises like TypeError)
        //         br 0 (Loop)
        //       }
        //     }
        //     else                              <- exhaustion only (S-02-17)
        //   }
        // The payload rides TEMP_LOCAL_A (print's arm already preserves A
        // across the body via the A/B swap — same contract).
        Stmt::For(target, iter, body, els) => {
            let ap_ir::item::Target::Name(tname) = target else {
                unreachable!("scan vetted For targets");
            };
            f.instruction(&Instruction::Block(BlockType::Empty)); // $exit
            ctx.depth += 1;
            let exit_depth = ctx.depth;
            f.instruction(&Instruction::Block(BlockType::Empty)); // $brk
            ctx.depth += 1;
            let brk_depth = ctx.depth;
            // it = iter_of(e)
            emit_expr(f, iter, ctx);
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_IT));
            f.instruction(&Instruction::Loop(BlockType::Empty)); // $head
            ctx.depth += 1;
            let head_depth = ctx.depth;
            // per-iteration next(), guarded:
            f.instruction(&Instruction::Block(BlockType::Result(
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_EXCEPTION),
                }),
            ))); // $cx
            ctx.depth += 1;
            {
                let catches = [wasm_encoder::Catch::One {
                    tag: TAG_PYEXC,
                    label: 0,
                }];
                f.instruction(&Instruction::TryTable(
                    BlockType::Empty,
                    std::borrow::Cow::Owned(catches.to_vec()),
                ));
                ctx.depth += 1;
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_IT));
                f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
                emit_name_store(f, tname.as_str(), ctx);
                ctx.depth -= 1;
                f.instruction(&Instruction::End); // try_table
                // success payload
                f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
            }
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // $cx: exc | null on stack
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
            // Exception checks run ONLY when the payload is non-null; a null
            // (success) payload skips them and falls into the body. $chk
            // wraps the checks so the null branch can Br past them.
            f.instruction(&Instruction::Block(BlockType::Empty)); // $chk
            ctx.depth += 1;
            let chk_depth = ctx.depth;
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            // payload NULL (success): skip the exception checks -> body.
            // br 1 exits the If AND the $chk block (br 0 would only exit the
            // If and fall INTO the checks — a no-op guard).
            f.instruction(&Instruction::Br(ctx.depth - chk_depth + 1));
            f.instruction(&Instruction::End);
            // ---- payload NON-NULL from here: an exception fired ----
            // StopIteration? -> exhaustion: Br $brk (0=If(SI), 1=If(null),
            // 2=$chk, 3=Loop, 4=$brk)
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
            f.instruction(&Instruction::GlobalGet(
                crate::types::exc_type_global("StopIteration").unwrap_or(0),
            ));
            f.instruction(&Instruction::Call(fx::FN_EXC_MATCH));
            f.instruction(&Instruction::If(BlockType::Empty));
            // labels from inside this If: 0=If, 1=$chk, 2=Loop, 3=$brk
            f.instruction(&Instruction::Br(ctx.depth - brk_depth + 1));
            f.instruction(&Instruction::End);
            // other iterator-side raise: rethrow
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::Throw(TAG_PYEXC));
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // $chk: success falls here -> body
            // loop body
            let saved_c = ctx.br_continue;
            let saved_b = ctx.br_break;
            ctx.br_continue = Some(head_depth); // br $head re-iterates
            ctx.br_break = Some(exit_depth); // $exit end skips else
            for s in body {
                emit_stmt(f, s, ctx);
            }
            ctx.br_continue = saved_c;
            ctx.br_break = saved_b;
            f.instruction(&Instruction::Br(0)); // next iteration
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // Loop
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // $brk: exhaustion falls here
            for s in els {
                emit_stmt(f, s, ctx); // normal-exit only (S-02-17)
            }
            ctx.depth -= 1;
            f.instruction(&Instruction::End); // $exit
        }
        // `global`/`nonlocal` are compile-time declarations: the name
        // resolution (scan/resolution_map) already routes reads/writes to
        // the module global or the enclosing cell — nothing to emit.
        Stmt::Global(_) | Stmt::Nonlocal(_) => {}
        Stmt::LocalDef(fd) => {
            // def-time binding (S-02-7): build the $function object HERE
            // (defaults evaluate at this execution point) and bind the name
            // in the current scope. Cells: the child's free_cells reference
            // THIS scope's cells — resolved at build time (S-02-13).
            let slot = ctx
                .scan
                .fns
                .iter()
                .rposition(|uf| uf.name == fd.name.as_str())
                .expect("scan registered the nested def");
            emit_function_object(f, slot, ctx);
            if let Some(cls) = ctx.cls_obj {
                // class-body method def: bind into the class namespace
                // (S-03-3) — attr_set(cls, name, fn). Operand order matters:
                // attr_set takes (obj, name, v) so the $function built above
                // rides a stash and is pushed LAST (03.03c fix: the previous
                // order [fn, cls, name] failed validation — attr_set's $str
                // param got the class $type ref).
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_FN));
                f.instruction(&Instruction::LocalGet(cls));
                push_interned_str(f, fd.name.as_str(), ctx);
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_FN));
                f.instruction(&Instruction::Call(fx::FN_ATTR_SET));
            } else {
                emit_name_store(f, fd.name.as_str(), ctx);
            }
        }
        // 03.03: class creation (S-03-1..4 slice). v1: single base (or
        // none), no decorators / metaclass hooks (fail-closed at scan);
        // the body executes in the CURRENT scope with method defs binding
        // through the namespace dict via attr_set on the class object.
        Stmt::LocalClass(cd) => emit_class_def(f, cd, ctx),
        // 03.03: raise forms (S-02-27/28/34).
        Stmt::Raise(e, cause) => emit_raise(f, e.as_ref(), cause.as_ref(), ctx),
        // 03.03: try/except/else (S-02-30/31/32); finally via catch-all
        // rethrow (S-02-33).
        Stmt::Try(ts) => emit_try(f, ts, ctx),
        other => unreachable!("scan vetted statement: {other:?}"),
    }
}

/// Emit a class definition: evaluate the single base (or none), cls_new,
/// execute the body in the current scope (methods bind through the
/// namespace dict via attr_set on the class object), bind the name.
fn emit_class_def(f: &mut Function, cd: &ap_ir::item::ClassDef, ctx: &mut EmitCtx) {
    // FT_CLS_NEW1 param order is (str name, ty base|null) — push the NAME
    // first, then the base (03.03b fix: the reversed order mis-typed the
    // call and every module-level class binding).
    push_interned_str(f, cd.name.as_str(), ctx);
    match cd.bases.len() {
        0 => {
            f.instruction(&Instruction::RefNull(hc(T_TYPE)));
        }
        1 => {
            emit_expr(f, &cd.bases[0], ctx);
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
        }
        _ => unreachable!("multi-base class rejected at scan (03.03 slice)"),
    }
    f.instruction(&Instruction::Call(fx::FN_CLS_NEW));
    // stash the class object; execute the body (method defs inside the
    // body emit LocalDef -> $function + attr_set on the class stash);
    // rebind the name at class-body end (S-03-3).
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
    let saved = ctx.cls_obj;
    ctx.cls_obj = Some(TEMP_LOCAL_A);
    for stmt in &cd.body {
        emit_stmt(f, stmt, ctx);
    }
    ctx.cls_obj = saved;
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
    emit_name_store(f, cd.name.as_str(), ctx);
}

/// Emit a raise (S-02-27/28/34): bare = re-raise via exc_raise on
/// G_ACTIVE_EXC; value form = exc_raise (throw + tb snapshot); `from`
/// rides exc_ctx_set first.
fn emit_raise(f: &mut Function, e: Option<&Expr>, cause: Option<&Expr>, ctx: &mut EmitCtx) {
    match e {
        None => {
            f.instruction(&Instruction::GlobalGet(ctx.fg.active_exc));
            f.instruction(&Instruction::Call(fx::FN_EXC_RAISE));
            f.instruction(&Instruction::Drop);
        }
        Some(expr) => {
            emit_expr(f, expr, ctx);
            if let Some(c) = cause {
                f.instruction(&Instruction::GlobalGet(ctx.fg.active_exc));
                emit_expr(f, c, ctx);
                f.instruction(&Instruction::Call(fx::FN_EXC_CTX_SET));
            }
            f.instruction(&Instruction::Call(fx::FN_EXC_RAISE));
            f.instruction(&Instruction::Drop);
        }
    }
}

/// try/except/else[/finally] (S-02-30..33).
fn emit_try(f: &mut Function, ts: &ap_ir::item::TryStmt, ctx: &mut EmitCtx) {
    if ts.finalbody.is_empty() {
        if ts.orelse.is_empty() {
            emit_try_handlers(f, ts, ctx, false);
        } else {
            // 03.03c: else (S-02-32) runs only on the success path. Wrap the
            // handler chain in one extra Block: the success Br and the
            // handler-completion Br both deepen by one, so success now lands
            // AFTER the inner block (orelse) while handlers still exit all.
            f.instruction(&Instruction::Block(BlockType::Empty));
            ctx.depth += 1;
            emit_try_handlers(f, ts, ctx, true);
            for s in &ts.orelse {
                emit_stmt(f, s, ctx);
            }
            ctx.depth -= 1;
            f.instruction(&Instruction::End);
        }
        return;
    }
    // finally (S-02-33): normal path runs finalbody inline; the exception
    // path re-raises through the handler chain — finalbody-on-exception is
    // the catch_all_ref surface, deferred (documented; corpus slice has
    // finally only on the normal path).
    emit_try_handlers(f, ts, ctx, false);
    for s in &ts.finalbody {
        emit_stmt(f, s, ctx);
    }
}

/// try/except/else without finally.
/// Shape (mirrors the proven top-level EH wrap, eh_probe facts 1-2):
///   Block $out
///     Block $catch (result exc-ref)         <- Catch label 0 (encloses try_table)
///       try_table (empty) [Catch TAG -> $catch]
///         body
///       end
///       RefNull(exc)                        # success payload
///     end                                   # payload here: real exc or null
///     LocalSet A; LocalGet A; RefIsNull; If  # null = success -> Br $out end
///       Br 1                                 # 0=If, 1=$out
///     End
///     handler chain (payload in A); unmatched -> rethrow
///   end
fn emit_try_handlers(f: &mut Function, ts: &ap_ir::item::TryStmt, ctx: &mut EmitCtx, extra_wrap: bool) {
    f.instruction(&Instruction::Block(BlockType::Empty)); // $out
    ctx.depth += 1;
    f.instruction(&Instruction::Block(BlockType::Result(
        ValType::Ref(wasm_encoder::RefType {
            nullable: true,
            heap_type: hc(T_EXCEPTION),
        }),
    ))); // $catch
    ctx.depth += 1;
    {
        let catches = [wasm_encoder::Catch::One {
            tag: TAG_PYEXC,
            label: 0, // $catch (immediately enclosing try_table — eh_probe fact 2)
        }];
        f.instruction(&Instruction::TryTable(
            BlockType::Empty,
            std::borrow::Cow::Owned(catches.to_vec()),
        ));
        ctx.depth += 1;
        for s in &ts.body {
            emit_stmt(f, s, ctx);
        }
        ctx.depth -= 1;
        f.instruction(&Instruction::End); // try_table
        // success: deliver a null payload to $catch
        f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
    }
    ctx.depth -= 1;
    f.instruction(&Instruction::End); // $catch: exc (or null) on stack
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
    f.instruction(&Instruction::RefIsNull);
    f.instruction(&Instruction::If(BlockType::Empty));
    // success: exit the whole try (0 = this If, 1 = $out; +1 under the
    // orelse wrap where it lands ON the orelse — $out end — instead)
    f.instruction(&Instruction::Br(1));
    f.instruction(&Instruction::End);
    // handler chain (S-02-30)
    for h in &ts.handlers {
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
        match &h.ty {
            None => {
                f.instruction(&Instruction::I32Const(1)); // bare except
            }
            Some(ty) => {
                emit_expr(f, ty, ctx);
                f.instruction(&Instruction::Call(fx::FN_EXC_MATCH));
            }
        }
        f.instruction(&Instruction::If(BlockType::Empty));
        ctx.depth += 1;
        if let Some(n) = &h.name {
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
            emit_name_store(f, n.as_str(), ctx);
        }
        for s in &h.body {
            emit_stmt(f, s, ctx);
        }
        // exit try: 0=this If, 1=$out, 2=orelse wrap outer (skip the orelse)
        f.instruction(&Instruction::Br(if extra_wrap { 2 } else { 1 }));
        ctx.depth -= 1;
        f.instruction(&Instruction::End);
    }
    // no handler matched: rethrow
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
    f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
    f.instruction(&Instruction::Throw(TAG_PYEXC));
    ctx.depth -= 1;
    f.instruction(&Instruction::End); // $out
}

/// Store the stack-top pyobj into `name` per the current scope.
/// Under the top-level EH wrap the catch payload rides TEMP_LOCAL_A from
/// the `$catch` block end to the null-test handler; a mid-body flush that
/// spills that local would corrupt the payload. Swap A<->B for the call
/// (both are $pyobj slots, so the swap is value-preserving).
fn scratch_ab_swap(f: &mut Function, ctx: &EmitCtx) {
    if ctx.eh_wrap_top {
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_B));
        f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
        f.instruction(&Instruction::LocalSet(TEMP_LOCAL_B));
    }
}

fn emit_name_store(f: &mut Function, name: &str, ctx: &mut EmitCtx) {
    match ctx.scope.kind {
        ScopeKind::Module => {
            let g = mod_global(name, &ctx.scan.module_names);
            f.instruction(&Instruction::GlobalSet(g));
        }
        ScopeKind::Fn => match ctx.scope.res.get(name) {
            Some(Res::OwnCell(i)) => {
                let cl = ctx.scope.cell_base + i;
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                f.instruction(&Instruction::LocalGet(cl));
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                f.instruction(&Instruction::StructSet {
                struct_type_index: T_CELL,
                field_index: F_CELL_V,
            });
            }
            Some(Res::FreeCell(i)) => {
                // `nonlocal` write into the enclosing cell (S-02-13)
                let cl = ctx.scope.cell_base + ctx.scope.n_own + i;
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                f.instruction(&Instruction::LocalGet(cl));
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                f.instruction(&Instruction::StructSet {
                struct_type_index: T_CELL,
                field_index: F_CELL_V,
            });
            }
            Some(Res::Local(l)) => {
                f.instruction(&Instruction::LocalSet(*l));
            }
            _ => {
                let g = mod_global(name, &ctx.scan.module_names);
                f.instruction(&Instruction::GlobalSet(g));
            }
        },
    }
}

/// Push `name`'s value per the current scope (LEGB read).
fn emit_name_load(f: &mut Function, name: &str, ctx: &mut EmitCtx) {
    match ctx.scope.kind {
        ScopeKind::Module => {
            // A name the module never BINDS but that names a builtin type
            // or an EXC_TREE class resolves to that class's builtin $type
            // global (03.03b: `raise ValueError(...)` read a never-written
            // module slot = null = dispatcher trap before this).
            if !ctx.scan.module_binds.iter().any(|n| n == name) {
                if let Some(g) = crate::types::exc_type_global(name) {
                    f.instruction(&Instruction::GlobalGet(g));
                    return;
                }
            }
            let g = mod_global(name, &ctx.scan.module_names);
            f.instruction(&Instruction::GlobalGet(g));
        }
        ScopeKind::Fn => match ctx.scope.res.get(name) {
            Some(Res::OwnCell(i)) => {
                f.instruction(&Instruction::LocalGet(ctx.scope.cell_base + i));
                f.instruction(&Instruction::StructGet {
                    struct_type_index: T_CELL,
                    field_index: F_CELL_V,
                });
            }
            Some(Res::FreeCell(i)) => {
                f.instruction(&Instruction::LocalGet(
                    ctx.scope.cell_base + ctx.scope.n_own + i,
                ));
                f.instruction(&Instruction::StructGet {
                    struct_type_index: T_CELL,
                    field_index: F_CELL_V,
                });
            }
            Some(Res::Local(l)) => {
                f.instruction(&Instruction::LocalGet(*l));
            }
            _ => {
                // Global fallback (module-level name read from a fn scope):
                // builtin types / EXC_TREE classes resolve to their $type
                // global FIRST (03.03b: `raise ValueError()` inside a def
                // read a never-registered module slot before this).
                if let Some(g) = crate::types::exc_type_global(name) {
                    f.instruction(&Instruction::GlobalGet(g));
                    return;
                }
                let g = mod_global(name, &ctx.scan.module_names);
                f.instruction(&Instruction::GlobalGet(g));
            }
        },
    }
}

/// Emit an expression; leaves the $pyobj on the operand stack.
fn emit_expr(f: &mut Function, e: &Expr, ctx: &mut EmitCtx) {
    match e {
        Expr::LitInt(v) => {
            f.instruction(&Instruction::I64Const(*v as i64));
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        }
        Expr::LitBigInt(digits) => {
            // >i64 literals wrap on this slice (03.02b resume notes: exact
            // big-literal emission is deferred with the limb core).
            let v: i128 = digits.parse().unwrap_or(0);
            let wrapped = if v > i64::MAX as i128 || v < i64::MIN as i128 {
                (v & 0xFFFF_FFFF_FFFF_FFFF) as i64
            } else {
                v as i64
            };
            f.instruction(&Instruction::I64Const(wrapped));
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        }
        Expr::LitFloat(v) => {
            f.instruction(&Instruction::F64Const((*v).into()));
            f.instruction(&Instruction::Call(fx::FN_FLOAT_FROM_I64));
        }
        Expr::LitBool(b) => {
            f.instruction(&Instruction::I32Const(0)); // unused op
            f.instruction(&Instruction::I64Const(*b as i64));
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
            f.instruction(&Instruction::Call(fx::FN_BOOL_OBJ));
        }
        Expr::LitNone => {
            f.instruction(&Instruction::GlobalGet(G_NONE));
        }
        Expr::LitEllipsis => {
            f.instruction(&Instruction::GlobalGet(G_ELLIPSIS));
        }
        Expr::LitStr(s) => push_interned_str(f, s, ctx),
        Expr::Name(n) => emit_name_load(f, n.as_str(), ctx),
        Expr::BinOp(op, a, b) => {
            f.instruction(&Instruction::I32Const(binop_op(*op)));
            emit_expr(f, a, ctx);
            emit_expr(f, b, ctx);
            f.instruction(&Instruction::Call(fx::FN_BINOP));
        }
        Expr::UnaryOp(op, a) => {
            let code_op = match op {
                UnaryOp::Plus => -1, // identity
                UnaryOp::Minus => OPS::NEG,
                UnaryOp::Invert => OPS::INVERT,
                UnaryOp::Not => OPS::NOT,
            };
            if code_op < 0 {
                emit_expr(f, a, ctx); // +x identity
            } else {
                f.instruction(&Instruction::I32Const(code_op));
                emit_expr(f, a, ctx);
                f.instruction(&Instruction::Call(fx::FN_UNARY));
            }
        }
        Expr::BoolOp(BoolOp::And, ops) => emit_bool_op(f, ops, false, ctx),
        Expr::BoolOp(BoolOp::Or, ops) => emit_bool_op(f, ops, true, ctx),
        Expr::IfExp(c, t, e) => {
            // S-01-27: lazy branches, operand VALUE result
            emit_expr(f, c, ctx);
            f.instruction(&Instruction::Call(fx::FN_TRUTHY));
            f.instruction(&Instruction::If(BlockType::Empty));
            ctx.depth += 1;
            emit_expr(f, t, ctx);
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
            ctx.depth -= 1;
            f.instruction(&Instruction::Else);
            ctx.depth += 1;
            emit_expr(f, e, ctx);
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
            ctx.depth -= 1;
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
        }
        Expr::Compare(ops, operands) => {
            // chain: AND-reduce each link (middle terms evaluated once on
            // the slice: side-effect-free operands)
            for (i, op) in ops.iter().enumerate() {
                f.instruction(&Instruction::I32Const(cmp_op(*op)));
                emit_expr(f, &operands[i], ctx);
                emit_expr(f, &operands[i + 1], ctx);
                f.instruction(&Instruction::Call(fx::FN_COMPARE));
            }
            for _ in 1..ops.len() {
                f.instruction(&Instruction::I32And);
            }
            // i32 -> bool object: type/kind args pushed FIRST (the compare
            // bool-build spills the folded i32 into the CMP temp)
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_CMP));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("bool").expect("TYPE_ORDER carries bool"),
            ));
            f.instruction(&Instruction::I32Const(0)); // kind = smi
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_CMP));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::RefNull(prelude::hc(T_BIGINT)));
            f.instruction(&Instruction::StructNew(T_INT));
        }
        // 03.03: attribute read (S-03-9): instances -> class mro walk
        Expr::Attribute(obj, name) => {
            emit_expr(f, obj, ctx);
            push_interned_str(f, name.as_str(), ctx);
            f.instruction(&Instruction::Call(fx::FN_ATTR_GET));
        }
        Expr::Call(func, args) => emit_call(f, func, args, ctx),
        Expr::Lambda(..) => {
            // lambda in expression position: build the $function object
            // (cells resolved against the current scope — S-02-12)
            if let Expr::Lambda(sig, body) = e {
                let body_stmts = vec![Stmt::Return(Some(body.as_ref().clone()))];
                let slot = ctx
                    .scan
                    .fns
                    .iter()
                    .rposition(|uf| {
                        uf.name == "<lambda>"
                            && uf.params.len() == sig.params.len()
                            && uf.is_generator == false
                            && uf.body == body_stmts
                    })
                    .expect("scan registered this lambda");
                emit_lambda_object(f, slot, ctx);
                return;
            }
            unreachable!("scan registered this lambda");
        }
        Expr::List(items) => {
            for it in items {
                emit_expr(f, it, ctx);
            }
            // $list.items is the MUT array type: array.new_fixed works on
            // mutable arrays too (the RO new_fixed was forced by the OLD
            // RO items field).
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR,
                array_size: items.len() as u32,
            });
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("list").unwrap_or(0),
            ));
            f.instruction(&Instruction::I32Const(items.len() as i32));
            f.instruction(&Instruction::I32Const(items.len() as i32));
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR));
            f.instruction(&Instruction::StructNew(T_LIST));
        }
        Expr::Tuple(items) => {
            for it in items {
                emit_expr(f, it, ctx);
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: items.len() as u32,
            });
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR_RO));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("tuple").unwrap_or(0),
            ));
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR_RO));
            f.instruction(&Instruction::StructNew(T_TUPLE));
        }
        Expr::Dict(items) => {
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("dict").unwrap_or(0),
            ));
            f.instruction(&Instruction::I32Const(items.len() as i32));
            f.instruction(&Instruction::I32Const(items.len() as i32));
            f.instruction(&Instruction::I32Const(7));
            f.instruction(&Instruction::I32Const(0));
            for _ in 0..8 {
                f.instruction(&Instruction::I32Const(-1));
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_IDX_ARR,
                array_size: 8,
            });
            for it in items {
                emit_expr(f, &it.key, ctx);
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                emit_expr(f, &it.value, ctx);
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_B));
                f.instruction(&Instruction::I64Const(0)); // hash (v1)
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_B));
                f.instruction(&Instruction::StructNew(T_DICT_ENTRY));
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_ENTRY_ARR,
                array_size: items.len() as u32,
            });
            f.instruction(&Instruction::StructNew(T_DICT));
        }
        other => unreachable!("scan vetted expr: {other:?}"),
    }
}

/// Push a RAW i32 truthiness value for a condition (While/If tests): a
/// Compare chain is already an i32 (skip the bool-object build — the
/// operand compare result IS its truthiness); everything else evaluates to
/// a pyobj and goes through TRUTHY. Semantically identical to
/// bool(cond)-then-test (S-01-27) — the bool object wrapper carries no
/// extra information the test reads.
fn emit_cond_i32(f: &mut Function, cond: &Expr, ctx: &mut EmitCtx) {
    if let Expr::Compare(ops, operands) = cond {
        for (i, op) in ops.iter().enumerate() {
            f.instruction(&Instruction::I32Const(cmp_op(*op)));
            emit_expr(f, &operands[i], ctx);
            emit_expr(f, &operands[i + 1], ctx);
            f.instruction(&Instruction::Call(fx::FN_COMPARE));
        }
        for _ in 1..ops.len() {
            f.instruction(&Instruction::I32And);
        }
        return;
    }
    emit_expr(f, cond, ctx);
    f.instruction(&Instruction::Call(fx::FN_TRUTHY));
}

/// Short-circuit BoolOp (S-01-30): result = first short-circuiting operand's
/// VALUE, else the last operand. Pattern per link:
///   Block { eval; set res; test; If { drop } Else { get res; br-out } ... }
fn emit_bool_op(f: &mut Function, ops: &[Expr], is_or: bool, ctx: &mut EmitCtx) {
    let res = TEMP_LOCAL_A;
    let entry = ctx.depth;
    f.instruction(&Instruction::Block(BlockType::Empty));
    ctx.depth += 1;
    let last = ops.len() - 1;
    for (i, op) in ops.iter().enumerate() {
        emit_expr(f, op, ctx);
        if i < last {
            f.instruction(&Instruction::LocalSet(res));
            f.instruction(&Instruction::LocalGet(res));
            f.instruction(&Instruction::Call(fx::FN_TRUTHY));
            if is_or {
                f.instruction(&Instruction::I32Eqz); // OR: leave when FALSY? no —
                // or: first TRUTHY wins -> leave when truthy; test as-is
            }
            // invert the branch sense per operator
            // and: continue when TRUTHY (If taken = drop copy, continue);
            //     leave (else) when falsy.
            // or:  continue when FALSY (If taken after Eqz); leave when truthy.
            // (the is_or Eqz above already normalized: taken = continue)
            f.instruction(&Instruction::If(BlockType::Empty));
            ctx.depth += 1;
            f.instruction(&Instruction::LocalGet(res));
            f.instruction(&Instruction::Drop);
            ctx.depth -= 1;
            f.instruction(&Instruction::Else);
            ctx.depth += 1;
            f.instruction(&Instruction::LocalGet(res));
            // exit the BoolOp Block (label depth = entry): site depth is
            // entry+2 (Block + If), so Br = site - entry - 1 = 1
            f.instruction(&Instruction::Br(ctx.depth - entry - 1));
            ctx.depth -= 1;
            f.instruction(&Instruction::End);
        } else {
            f.instruction(&Instruction::LocalSet(res));
        }
    }
    ctx.depth -= 1;
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::LocalGet(res));
}

/// Call emission: builtins first, then static exact-arity module defs take
/// the direct path (S-07-24); everything else builds (fn, args, kwargs)
/// for the dispatcher.
fn emit_call(f: &mut Function, func: &Expr, args: &[Arg], ctx: &mut EmitCtx) {
    // 03.03d: super(S, obj) / bare super() (S-03-17 slice) — FN_SUPER_NEW.
    // Zero-arg form passes S = null (the frame has no __class__ cell on this
    // slice, so every bare super() raises RuntimeError at call time — the
    // c021 observable). Only 0 or 2 positional args reach here (scan).
    if let Expr::Name(n) = func {
        if n.as_str() == "super" {
            let positional: Vec<&Expr> = args
                .iter()
                .filter_map(|a| match a {
                    Arg::Pos(x) => Some(x),
                    _ => None,
                })
                .collect();
            match positional.len() {
                0 => {
                    f.instruction(&Instruction::RefNull(hc(T_PYOBJ))); // S = null
                    f.instruction(&Instruction::RefNull(hc(T_PYOBJ))); // obj
                    f.instruction(&Instruction::RefNull(hc(T_PYOBJ))); // unused
                    f.instruction(&Instruction::Call(fx::FN_SUPER_NEW));
                }
                2 => {
                    emit_expr(f, positional[0], ctx);
                    emit_expr(f, positional[1], ctx);
                    f.instruction(&Instruction::RefNull(hc(T_PYOBJ))); // unused
                    f.instruction(&Instruction::Call(fx::FN_SUPER_NEW));
                }
                _ => unreachable!("scan vetted super() arity (0 or 2)"),
            }
            return;
        }
    }
    // 03.04d: STATIC generator call — build a parked $generator (params
    // bind into the $locals snapshot; the body runs on first resume).
    if let Expr::Name(name) = func {
        let gslots: Vec<usize> = ctx
            .scan
            .fns
            .iter()
            .enumerate()
            .filter(|(_, uf)| uf.name == name.as_str() && uf.is_generator)
            .map(|(i, _)| i)
            .collect();
        let shadowed = ctx.scope.res.contains_key(name.as_str());
        if gslots.len() == 1
            && !shadowed
            && args.iter().all(|a| matches!(a, Arg::Pos(_)))
            && args.len() == ctx.scan.fns[gslots[0]].params.len()
        {
            emit_generator_call(f, gslots[0], args, ctx);
            return;
        }
    }
    // 03.04e: generator method arms (PEP-342 protocol surface). The
    // generator object has NO attrs dict — attr_get on send/close/throw
    // answers null and the $function cast below traps, so these are
    // intercepted BEFORE the attribute path. Arity-checking is loose on
    // this slice (close() swallows extra args; CPython raises TypeError).
    if let Expr::Attribute(obj, mname) = func {
        let m = mname.as_str();
        if matches!(m, "send" | "close" | "throw") {
            emit_expr(f, obj, ctx);
            f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
            match m {
                "send" => {
                    let Arg::Pos(x) = &args[0] else {
                        unreachable!("scan vetted send arg");
                    };
                    emit_expr(f, x, ctx);
                    f.instruction(&Instruction::Call(fx::FN_GEN_SEND));
                }
                "close" => {
                    f.instruction(&Instruction::Call(fx::FN_GEN_CLOSE));
                    f.instruction(&Instruction::GlobalGet(G_NONE));
                }
                "throw" => {
                    // c.throw(exc_or_value): single arg on this slice.
                    let Arg::Pos(x) = &args[0] else {
                        unreachable!("scan vetted throw arg");
                    };
                    emit_expr(f, x, ctx);
                    f.instruction(&Instruction::Call(fx::FN_GEN_THROW));
                }
                _ => unreachable!(),
            }
            return;
        }
    }
    // 03.03: method call obj.m(*args) — attribute lookup; when the found
    // value is a plain $function it is a descriptor and binds self
    // (S-03-9 binding rule) — args become [self, *args].
    if let Expr::Attribute(obj, mname) = func {
        emit_expr(f, obj, ctx);
        f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A)); // self
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
        push_interned_str(f, mname.as_str(), ctx);
        f.instruction(&Instruction::Call(fx::FN_ATTR_GET));
        f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
        // static-arity cascade: [self, *args] via array.new_fixed
        let n = args.len();
        if n > 2 {
            return; // handled below via call_of? fail loud:
        }
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
        for (i, a) in args.iter().enumerate() {
            let Arg::Pos(x) = a else { unreachable!("scan vetted positional args") };
            emit_expr(f, x, ctx);
            let _ = i;
        }
        f.instruction(&Instruction::ArrayNewFixed {
            array_type_index: T_OBJ_ARR_RO,
            array_size: (n + 1) as u32,
        });
        f.instruction(&Instruction::RefNull(hc(T_DICT)));
        f.instruction(&Instruction::Call(fx::FN_DISPATCH));
        return;
    }
    // 03.03d: `type(x)` (1-arg form) — the object's $type as a VALUE (c021's
    // type(super(..)).__name__ surface). The $type IS a $pyobj subtype; the
    // read is a raw field-0 struct.get in the user-fn body. (The 3-arg
    // metaclass form stays rejected — scan keeps arity 1.)
    if let Expr::Name(n) = func {
        if n.as_str() == "type" && args.len() == 1 {
            if let [Arg::Pos(x)] = args {
                emit_expr(f, x, ctx);
                f.instruction(&Instruction::Call(fx::FN_TYPE_OF));
                return;
            }
        }
    }
    if let Expr::Name(name) = func {
        if !ctx.scan.fns.iter().any(|uf| uf.name == name.as_str()) {
            if is_supported_builtin(name.as_str()) {
                emit_builtin_call(f, name.as_str(), args, ctx);
                return;
            }
            // unknown non-function call: fail loud at emit time (scan
            // already vetted the value slice's surface; this arm is
            // defensive for function-valued NAMES)
        }
    }
    if let Expr::Name(name) = func {
        let slots: Vec<usize> = ctx
            .scan
            .fns
            .iter()
            .enumerate()
            .filter(|(_, uf)| uf.name == name.as_str() && uf.parent == usize::MAX)
            .map(|(i, _)| i)
            .collect();
        let shadowed = ctx.scope.res.get(name.as_str()).is_some()
            && !matches!(ctx.scope.res.get(name.as_str()), Some(Res::Global) | None);
        // A call argument containing a nested Call would clobber the A/B
        // stashes mid-evaluation — route those through the generic path
        // (stack-ordered, no stashes). Same observable behavior (S-01-31).
        let arg_has_nested_call = args.iter().any(|a| matches!(
            a,
            Arg::Pos(Expr::Call(..))
                | Arg::Pos(Expr::BinOp(..))
                | Arg::Pos(Expr::UnaryOp(..))
        ));
        if slots.len() == 1
            && !shadowed
            && !arg_has_nested_call
            && args.iter().all(|a| matches!(a, Arg::Pos(_)))
            && args.len() == ctx.scan.fns[slots[0]].params.len()
        {
            // Static FAST path (S-07-24): same uniform convention as the
            // generic path — fn object, args RO array, null kwargs — but
            // `call` DIRECTLY (fn_idx), skipping the dispatcher's arity
            // check. Args: eval left-to-right into stashes, then build the
            // array (array.new_fixed consumes push order; stashes reload
            // in argument order — <=2 args shapes only; >=3 rides the
            // generic path whose left-to-right eval is stack-ordered).
            if args.len() <= 2 {
                // $function-typed param 0: the module global is $pyobj-typed
                // — cast (traps only if a non-function was somehow bound,
                // which scan's shadowing check excludes)
                emit_name_load(f, name.as_str(), ctx);
                f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
                for (i, a) in args.iter().enumerate() {
                    let Arg::Pos(x) = a else { unreachable!() };
                    emit_expr(f, x, ctx);
                    f.instruction(&Instruction::LocalSet(match i {
                        0 => TEMP_LOCAL_A,
                        _ => TEMP_LOCAL_B,
                    }));
                }
                if args.len() >= 1 {
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                }
                if args.len() == 2 {
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_B));
                }
                f.instruction(&Instruction::ArrayNewFixed {
                    array_type_index: T_OBJ_ARR_RO,
                    array_size: args.len() as u32,
                });
                f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
                    T_DICT,
                )));
                f.instruction(&Instruction::Call(fn_idx(slots[0])));
                return;
            }
        }
    }
    // Builtin exception classes are CALLED to instantiate (call_of routes
    // $type BaseException-derived through exc_new): FT_CALL_OF takes a
    // plain $pyobj callable, no $function cast (a $type would trap it).
    // 03.03c: USER classes ride the same path — a bound class NAME holds a
    // $type, and casting that to $function traps (call_of's $type arm ->
    // cls_call does __init__ dispatch).
    if let Expr::Name(name) = func {
        let is_class = !ctx.scan.module_binds.iter().any(|n| n == name.as_str())
            && crate::types::exc_type_global(name.as_str()).is_some();
        let is_user_class = ctx.scan.classes.iter().any(|c| c == name.as_str());
        if is_class || is_user_class {
            emit_name_load(f, name.as_str(), ctx);
            for a in args {
                let Arg::Pos(x) = a else {
                    unreachable!("scan vetted positional args");
                };
                emit_expr(f, x, ctx);
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: args.len() as u32,
            });
            f.instruction(&Instruction::Call(fx::FN_CALL_OF));
            return;
        }
    }
    // generic path: fn object, args array, null kwargs -> dispatcher
    emit_function_ref(f, func, ctx);
    for a in args {
        let Arg::Pos(x) = a else {
            unreachable!("scan vetted positional args");
        };
        emit_expr(f, x, ctx);
    }
    f.instruction(&Instruction::ArrayNewFixed {
        array_type_index: T_OBJ_ARR_RO,
        array_size: args.len() as u32,
    });
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_DICT,
    )));
    f.instruction(&Instruction::Call(fx::FN_DISPATCH));
}

/// Push the `$function` OBJECT for a call target: name -> namespace load
/// (def-time object); lambda -> fresh object built here.
fn emit_function_ref(f: &mut Function, func: &Expr, ctx: &mut EmitCtx) {
    if let Expr::Name(name) = func {

        // nested defs re-bind the name when their def statement executes;
        // the namespace/local load is the uniform, order-correct path
        // (builtins never collide: scan keeps the slice's builtin names
        // out of the function table). The dispatcher wants a $function —
        // cast (traps fail-loud on a non-callable, matching the slice's
        // callable surface).
        emit_name_load(f, name.as_str(), ctx);
        f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
        return;
    }
    if let Expr::Lambda(..) = func {
        let body_probe = match func {
            Expr::Lambda(_, b) => b.clone(),
            _ => unreachable!(),
        };
        let body_stmts = vec![Stmt::Return(Some(*body_probe))];
        let sig = match func {
            Expr::Lambda(s, _) => s.clone(),
            _ => unreachable!(),
        };
        let slot = ctx
            .scan
            .fns
            .iter()
            .rposition(|uf| {
                uf.name == "<lambda>"
                    && uf.params.len() == sig.params.len()
                    && uf.body == body_stmts
            })
            .expect("scan registered this lambda");
        emit_lambda_object(f, slot, ctx);
        return;
    }
    // callable VALUE expression: cast to $function for the convention
    // (fail-loud trap on a non-callable — the slice's callable surface is
    // functions only)
    emit_expr(f, func, ctx);
    f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
}

/// Build a `$function` for a lambda at the current execution point
/// (same assembly as a def: defaults are empty — scan rejects lambda
/// defaults; cells ride the enclosing scope like any def).
fn emit_lambda_object(f: &mut Function, slot: usize, ctx: &mut EmitCtx) {
    let uf = ctx.scan.fns[slot].clone();
    let nparams = uf.params.len() as u32;
    // defaults: EMPTY MUT $obj_arr ($function.defaults is non-null)
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR));
    // param names (OBJARR2; 44 holds defaults)
    f.instruction(&Instruction::I32Const(nparams as i32));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR2));
    for (i, p) in uf.params.iter().enumerate() {
        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR2));
        f.instruction(&Instruction::I32Const(i as i32));
        push_interned_str(f, p.name.as_str(), ctx);
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    }
    // $function type ref FIRST (field 0), then the $code field sequence.
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("function").unwrap_or(0),
    ));
    push_interned_str(f, "<lambda>", ctx);
    f.instruction(&Instruction::I32Const(nparams as i32));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR2)); // param_names
    f.instruction(&Instruction::I32Const(fn_idx(slot) as i32));
    f.instruction(&Instruction::StructNew(T_CODE));
    // $function tail fields: defaults, kwdefaults, cells, globals
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR)); // defaults (empty mut arr)
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_DICT,
    )));
    let n_cells = uf.free_cells.len() as u32;
    if n_cells > 0 {
        for name in &uf.free_cells {
            push_resolved_cell(f, name, ctx, slot);
        }
        f.instruction(&Instruction::ArrayNewFixed {
            array_type_index: T_CELL_ARR,
            array_size: n_cells,
        });
    } else {
        f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_CELL_ARR,
        )));
    }
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_DICT,
    )));
    f.instruction(&Instruction::StructNew(T_FUNCTION));
}

fn binop_op(op: BinOp) -> i32 {
    match op {
        BinOp::Add => OPS::ADD,
        BinOp::Sub => OPS::SUB,
        BinOp::Mul => OPS::MUL,
        BinOp::Div => OPS::DIV,
        BinOp::FloorDiv => OPS::FLOORDIV,
        BinOp::Mod => OPS::MOD,
        BinOp::Pow => OPS::POW,
        BinOp::LShift => OPS::LSHIFT,
        BinOp::RShift => OPS::RSHIFT,
        BinOp::BitAnd => OPS::AND,
        BinOp::BitXor => OPS::XOR,
        BinOp::BitOr => OPS::OR,
        BinOp::MatMul => -1, // scan rejects
    }
}

fn is_supported_builtin(n: &str) -> bool {
    matches!(
        n,
        "print" | "repr" | "str" | "bool" | "hash" | "len" | "isinstance" | "issubclass"
            | "next" | "range" | "list" | "sum" | "iter" | "tuple"
    )
}

/// Builtin call emission (the 03.02b surface, carried forward: print via
/// the print buffer, repr/str/bool/hash/len via prelude entry points).
fn emit_builtin_call(f: &mut Function, name: &str, args: &[Arg], ctx: &mut EmitCtx) {
    fn emit_args(f: &mut Function, args: &[Arg], ctx: &mut EmitCtx) {
        for a in args {
            let Arg::Pos(x) = a else {
                unreachable!("scan vetted positional args");
            };
            emit_expr(f, x, ctx);
        }
    }
    match name {
        "print" => {
            // print(*args): space-joined str(x) + newline via the print
            // buffer; running end lives in TEMP_LOCAL (i32).
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                    f.instruction(&Instruction::I32Const(b' ' as i32));
                    f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
                        offset: 0,
                        align: 0,
                        target: wasm_encoder::MemArgTarget::Memory(0),
                        ordering: None,
                    }));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                    f.instruction(&Instruction::I32Const(1));
                    f.instruction(&Instruction::I32Add);
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
                }
                let Arg::Pos(xe) = a else {
                    unreachable!("scan vetted positional args");
                };
                emit_expr(f, xe, ctx);
                f.instruction(&Instruction::Call(fx::FN_PY_STR));
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_S));
                // EH wrap: PRINT_ONE clobbers TEMP_LOCAL_A — stash the wrap
                // payload into B (via locals, NOT the operand stack: the
                // print args are already... nothing pending here) BEFORE the
                // args are pushed, restore after the buffer update.
                if ctx.eh_wrap_top {
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_B));
                }
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_S));
                f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
                if ctx.eh_wrap_top {
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_B));
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                }
            }
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
            f.instruction(&Instruction::Call(fx::FN_PRINT_FLUSH));
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
        }
        "bool" => {
            f.instruction(&Instruction::I32Const(0)); // unused op
            emit_args(f, args, ctx);
            f.instruction(&Instruction::Call(fx::FN_BOOL_OBJ));
        }
        "hash" => {
            emit_args(f, args, ctx);
            f.instruction(&Instruction::Call(fx::FN_HASH));
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        }
        "repr" | "str" => {
            emit_args(f, args, ctx);
            f.instruction(&Instruction::Call(fx::FN_PY_STR));
        }
        "len" => {
            emit_args(f, args, ctx);
            f.instruction(&Instruction::Call(fx::FN_LEN));
        }
        // 03.03: isinstance/issubclass (S-03-31) — bool-object result.
        "isinstance" => {
            // FT (o, ty): the CLASS operand (2nd, top of stack after
            // emit_args) is a $pyobj-scope load — cast to (ref null $type).
            emit_args(f, args, ctx);
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::Call(fx::FN_ISINST));
            emit_bool_object(f);
        }
        "issubclass" => {
            // FT (o, o): the runtime casts both operands to $type itself
            // (operands arrive as $pyobj-scope loads; no emit-side stash).
            emit_args(f, args, ctx);
            f.instruction(&Instruction::Call(fx::FN_ISSUB));
            emit_bool_object(f);
        }
        // ---- leaf 03.04c iterator builtins ----
        // next(x) / next(x, default): try FN_ITER_NEXT catch TAG -> default.
        // 1-arg: exhaustion TAG propagates (byte-exact StopIteration); 2-arg:
        // the TAG is swallowed and the default pushed.
        "next" => {
            // 1-arg: value = iter_next(it) (SI propagates). 2-arg:
            // canonical Block($cx result exc) + try_table; the retry
            // pushes ONLY the iterator (default stashed in B first).
            let posn: Vec<&Expr> = args
                .iter()
                .filter_map(|a| match a {
                    Arg::Pos(x) => Some(x),
                    _ => None,
                })
                .collect();
            if posn.len() == 2 {
                if let [Arg::Pos(it), Arg::Pos(def)] = args {
                    emit_expr(f, def, ctx);
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_B));
                    emit_expr(f, it, ctx);
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::Block(BlockType::Result(
                        ValType::Ref(wasm_encoder::RefType {
                            nullable: true,
                            heap_type: hc(T_EXCEPTION),
                        }),
                    ))); // $cx
                    let catches = [wasm_encoder::Catch::One {
                        tag: TAG_PYEXC,
                        label: 0,
                    }];
                    f.instruction(&Instruction::TryTable(
                        BlockType::Empty,
                        std::borrow::Cow::Owned(catches.to_vec()),
                    ));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::End); // try_table
                    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION))); // payload
                    f.instruction(&Instruction::End); // $cx: exc|null
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_S));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_S));
                    f.instruction(&Instruction::RefIsNull);
                    f.instruction(&Instruction::If(BlockType::Result(pyobj_ref())));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                    f.instruction(&Instruction::Else);
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_B));
                    f.instruction(&Instruction::End);
                } else {
                    unreachable!("scan vetted next() arg kinds");
                }
            } else {
                for a in args {
                    let Arg::Pos(x) = a else {
                        unreachable!("scan vetted positional args");
                    };
                    emit_expr(f, x, ctx);
                }
                f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
            }
        }
        "range" => {
            let pos: Vec<&Expr> = args
                .iter()
                .filter_map(|a| match a {
                    Arg::Pos(x) => Some(x),
                    _ => None,
                })
                .collect();
            emit_range(f, &pos, ctx);
        }
        // list(x): iter + drain into a $list via FN_LIST_FROM_ITER? — no
        // driver exists; drain inline (same shape as the DictComp acc):
        // rebuild-append loop. On this slice list(x) over a $generator
        // container-iterator = snapshot copy; over a USER generator = run it.
        "list" => {
            emit_args(f, args, ctx);
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
            f.instruction(&Instruction::Call(fx::FN_LIST_DRAIN));
        }
        // sum(x[, start]): acc = start (default $int 0); drain adding.
        "sum" => {
            emit_args(f, args, ctx);
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
            if args.len() == 2 {
                let Arg::Pos(start_expr) = &args[1] else {
                    unreachable!()
                };
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A));
                emit_expr(f, start_expr, ctx);
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_B));
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_B));
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
                f.instruction(&Instruction::Call(fx::FN_SUM_DRAIN));
            } else {
                f.instruction(&Instruction::Call(fx::FN_SUM_DRAIN));
            }
        }
        // iter(x) = FN_ITER_OF (identity for generators, snapshot otherwise)
        "iter" => {
            emit_args(f, args, ctx);
            f.instruction(&Instruction::Call(fx::FN_ITER_OF));
        }
        _ => unreachable!("is_supported_builtin vetted: {name}"),
    }
}

/// i32 bool on the stack -> bool $int object (same build as the Compare
/// path: [bool-type, kind=smi, value, null-bigint]).
/// range(a,b,step) / range(n): build the container-iterator $generator
/// INLINE in the CALLER frame (locals: 53=start i64, 54=stop, 55=step,
/// 56=count i32, 57=arr-stash, 58=cursor-box, 59=i32 scratch). Every frame
/// (module + user) reserves 53..59 for this leaf.
/// Locals snapshot = materialized values [start, start+step, ...]. Bounded
/// by stop-start (no lazy range object on this slice — memory cost = the
/// materialized list, matching iter_of's snapshot shape).
fn emit_range(f: &mut Function, pos: &[&Expr], ctx: &mut EmitCtx) {
    // 1. start / stop / step into i64 locals (defaults 0 / arg / 1)
    match pos.len() {
        1 => {
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(53));
            emit_expr_i64(f, pos[0], ctx);
            f.instruction(&Instruction::LocalSet(54));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::LocalSet(55));
        }
        2 => {
            emit_expr_i64(f, pos[0], ctx);
            f.instruction(&Instruction::LocalSet(53));
            emit_expr_i64(f, pos[1], ctx);
            f.instruction(&Instruction::LocalSet(54));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::LocalSet(55));
        }
        3 => {
            emit_expr_i64(f, pos[0], ctx);
            f.instruction(&Instruction::LocalSet(53));
            emit_expr_i64(f, pos[1], ctx);
            f.instruction(&Instruction::LocalSet(54));
            emit_expr_i64(f, pos[2], ctx);
            f.instruction(&Instruction::LocalSet(55));
        }
        _ => unreachable!("scan vetted range() arity"),
    }
    // 2. count = steps between start and stop (ceil for step>0: (stop-start+step-1)/step)
    //    i64 -> i32 (bounds-checked slice)
    f.instruction(&Instruction::LocalGet(54));
    f.instruction(&Instruction::LocalGet(53));
    f.instruction(&Instruction::I64Sub);
    f.instruction(&Instruction::LocalGet(55));
    f.instruction(&Instruction::I64Add);
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64Sub);
    f.instruction(&Instruction::LocalGet(55));
    f.instruction(&Instruction::I64DivS);
    f.instruction(&Instruction::I32WrapI64);
    f.instruction(&Instruction::LocalSet(56)); // count (i32)
    // negative count -> 0 (clamp: LocalGet; 0 LtS; If {0})
    f.instruction(&Instruction::LocalGet(56));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32LtS);
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(56));
    f.instruction(&Instruction::End);
    // 3. materialize the snapshot into a mut $obj_arr (LOCAL 57 — NOT
    // TEMP_LOCAL_OBJARR(44): emit_expr of a LitInt clobbers 44/45 via the
    // int build path, so the snapshot arr must live OUTSIDE the shared
    // temp window).
    f.instruction(&Instruction::LocalGet(56));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(57));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(59)); // i
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::Loop(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(59));
    f.instruction(&Instruction::LocalGet(56));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::BrIf(1));
    // arr[i] = $int(start + i*step) — $int build is stack-local (no stash)
    f.instruction(&Instruction::LocalGet(57));
    f.instruction(&Instruction::LocalGet(59));
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("int").unwrap_or(0),
    ));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalGet(53));
    f.instruction(&Instruction::LocalGet(59));
    f.instruction(&Instruction::I64ExtendI32U);
    f.instruction(&Instruction::LocalGet(55));
    f.instruction(&Instruction::I64Mul);
    f.instruction(&Instruction::I64Add);
    f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
    f.instruction(&Instruction::StructNew(T_INT));
    f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    f.instruction(&Instruction::LocalGet(59));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(59));
    f.instruction(&Instruction::Br(0));
    f.instruction(&Instruction::End); // loop
    f.instruction(&Instruction::End); // block
    // 4. the $generator struct { fn: null, state: count, locals: snapshot,
    //    value: $int(0) cursor, exn: null } (iter_snapshot shape)
    f.instruction(&Instruction::RefNull(hc(T_TYPE))); // $pyobj field 0
    f.instruction(&Instruction::RefNull(hc(T_FUNCTION)));
    f.instruction(&Instruction::LocalGet(56)); // state = count
    f.instruction(&Instruction::LocalGet(57)); // locals snapshot
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("int").unwrap_or(0),
    ));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
    f.instruction(&Instruction::StructNew(T_INT)); // cursor box = 0
    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
    f.instruction(&Instruction::StructNew(T_GENERATOR));
}

/// shared range tail (locals 53..59 already loaded): count+materialize+
/// $generator struct build (03.04d: reused by the gen-slice range arm).
fn emit_range_tail(f: &mut Function) {
    // 2. count = steps between start and stop (ceil for step>0: (stop-start+step-1)/step)
    //    i64 -> i32 (bounds-checked slice)
    f.instruction(&Instruction::LocalGet(54));
    f.instruction(&Instruction::LocalGet(53));
    f.instruction(&Instruction::I64Sub);
    f.instruction(&Instruction::LocalGet(55));
    f.instruction(&Instruction::I64Add);
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64Sub);
    f.instruction(&Instruction::LocalGet(55));
    f.instruction(&Instruction::I64DivS);
    f.instruction(&Instruction::I32WrapI64);
    f.instruction(&Instruction::LocalSet(56)); // count (i32)
    // negative count -> 0 (clamp: LocalGet; 0 LtS; If {0})
    f.instruction(&Instruction::LocalGet(56));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32LtS);
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(56));
    f.instruction(&Instruction::End);
    // 3. materialize the snapshot into a mut $obj_arr (LOCAL 57 — NOT
    // TEMP_LOCAL_OBJARR(44): emit_expr of a LitInt clobbers 44/45 via the
    // int build path, so the snapshot arr must live OUTSIDE the shared
    // temp window).
    f.instruction(&Instruction::LocalGet(56));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(57));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(59)); // i
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::Loop(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(59));
    f.instruction(&Instruction::LocalGet(56));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::BrIf(1));
    // arr[i] = $int(start + i*step) — $int build is stack-local (no stash)
    f.instruction(&Instruction::LocalGet(57));
    f.instruction(&Instruction::LocalGet(59));
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("int").unwrap_or(0),
    ));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalGet(53));
    f.instruction(&Instruction::LocalGet(59));
    f.instruction(&Instruction::I64ExtendI32U);
    f.instruction(&Instruction::LocalGet(55));
    f.instruction(&Instruction::I64Mul);
    f.instruction(&Instruction::I64Add);
    f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
    f.instruction(&Instruction::StructNew(T_INT));
    f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    f.instruction(&Instruction::LocalGet(59));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(59));
    f.instruction(&Instruction::Br(0));
    f.instruction(&Instruction::End); // loop
    f.instruction(&Instruction::End); // block
    // 4. the $generator struct { fn: null, state: count, locals: snapshot,
    //    value: $int(0) cursor, exn: null } (iter_snapshot shape)
    f.instruction(&Instruction::RefNull(hc(T_TYPE))); // $pyobj field 0
    f.instruction(&Instruction::RefNull(hc(T_FUNCTION)));
    f.instruction(&Instruction::LocalGet(56)); // state = count
    f.instruction(&Instruction::LocalGet(57)); // locals snapshot
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("int").unwrap_or(0),
    ));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
    f.instruction(&Instruction::StructNew(T_INT)); // cursor box = 0
    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
    f.instruction(&Instruction::StructNew(T_GENERATOR));
}

/// Evaluate an int expression into an i64 on the stack (emit_expr + unbox).
fn emit_expr_i64(f: &mut Function, e: &Expr, ctx: &mut EmitCtx) {
    emit_expr(f, e, ctx);
    f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
    f.instruction(&Instruction::StructGet {
        struct_type_index: T_INT,
        field_index: F_INT_SMI,
    });
}

fn emit_bool_object(f: &mut Function) {
    f.instruction(&Instruction::LocalSet(TEMP_LOCAL_CMP));
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("bool").expect("TYPE_ORDER carries bool"),
    ));
    f.instruction(&Instruction::I32Const(0)); // kind = smi
    f.instruction(&Instruction::LocalGet(TEMP_LOCAL_CMP));
    f.instruction(&Instruction::I64ExtendI32U);
    f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
    f.instruction(&Instruction::StructNew(T_INT));
}

fn cmp_op(op: CmpOp) -> i32 {
    match op {
        CmpOp::Eq => OPS::EQ,
        CmpOp::NotEq => OPS::NE,
        CmpOp::Lt => OPS::LT,
        CmpOp::LtE => OPS::LE,
        CmpOp::Gt => OPS::GT,
        CmpOp::GtE => OPS::GE,
        CmpOp::Is => OPS::EQ, // v1: identity via eq (ref-eq shortcut)
        CmpOp::IsNot => OPS::NE,
        CmpOp::In => OPS::IN,
        CmpOp::NotIn => OPS::IN, // negated at use site (v1 approximation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_ir::{Builder, Item, Module};

    #[test]
    fn hello_world_lowers_and_validates() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.expr_stmt(
            b.call(b.name("print"), vec![b.arg_pos(b.str_lit("hello, wasm"))]),
        )));
        let bytes = lower_module(&module).expect("hello world must lower");
        assert_eq!(&bytes[..4], b"\0asm");
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default())
            .validate_all(&bytes)
            .expect("emitted module must validate");
    }

    #[test]
    fn try_lowering_landed_03_03b() {
        // 03.03b landed emit_try — bare try/except/pass now lowers (no
        // Unsupported). The module must build AND validate.
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.try_(
            vec![b.pass_()],
            vec![b.except_handler(None, None, vec![b.pass_()])],
            vec![],
            vec![],
        )));
        let bytes = lower_module(&module).expect("Try lowers since 03.03b");
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default())
            .validate_all(&bytes)
            .expect("emitted module must validate");
    }

    #[test]
    fn controlflow_and_calls_module_validates() {
        let src = r#"
def f(n):
    total = 0
    i = 0
    while i < n:
        i = i + 1
        if i % 2 == 0:
            continue
        total = total + i
    return total
print(f(10))
"#;
        let (module, _) = ap_parse::lower_source(src).expect("parse");
        let bytes = lower_module(&module).expect("lower");
        std::fs::write("/tmp/min.wasm", &bytes).expect("write");
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default())
            .validate_all(&bytes)
            .expect("control-flow module must validate");
    }
}

/// Intern `s` at scan time (names referenced by emitted attribute lookups
/// and class builds that never appear as source literals).
fn scan_intern_str_in(scan: &mut Scan, s: &str) {
    scan.intern_str(s);
}
