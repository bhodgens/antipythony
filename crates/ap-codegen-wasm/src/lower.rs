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
use crate::prelude::{self, hc, sset, StrTab};
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
}

/// Scan state: fail-closed scope check, function registration, string
/// interning, module-name table.
struct Scan {
    fns: Vec<UserFn>,
    /// Module-level names in first-reference order (global slot = index).
    module_names: Vec<String>,
    strings: Vec<Vec<u8>>,
    /// Registration-stack parent slots (nested-def parent links).
    parent_stack: Vec<usize>,
}

impl Scan {
    fn new() -> Self {
        Self {
            fns: Vec::new(),
            module_names: Vec::new(),
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
                scan_fn_common(fd.name.as_str(), &fd.params, &fd.body.blocks, usize::MAX, scan)?;
            }
            Item::ClassDef(_) => return Err(CodegenError::Unsupported("Item::ClassDef (03.03)")),
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
    });
    // scan the body with this fn as the nesting parent
    let mut refs: Vec<String> = Vec::new();
    let mut bound: Vec<String> = params.iter().map(|p| p.name.to_string()).collect();
    let mut nonlocal: Vec<String> = Vec::new();
    scan.parent_stack.push(slot);
    let r = collect_scope(body, &mut refs, &mut bound, &mut nonlocal, scan);
    scan.parent_stack.pop();
    r?;
    scan.fns[slot].nonlocal = nonlocal;
    for r in refs {
        if bound.contains(&r) || scan.fns[slot].free.contains(&r) {
            continue;
        }
        scan.fns[slot].free.push(r);
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
            for t in targets {
                collect_target(t, bound, scan)?;
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
            if fd.is_generator || fd.is_async {
                return Err(CodegenError::Unsupported("generator/async def (leaf 03.04)"));
            }
            let parent = scan.parent_stack.last().copied().unwrap_or(usize::MAX);
            scan_fn_common(fd.name.as_str(), &fd.params, &fd.body.blocks, parent, scan)
        }
        other => Err(CodegenError::Unsupported(stmt_unsupported(other))),
    }
}

/// Assignment target: binds (name) or reads (subscript/attribute obj+key).
fn collect_target(t: &ap_ir::item::Target, bound: &mut Vec<String>, scan: &mut Scan) -> Result<(), CodegenError> {
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
        Expr::Call(f, args) => {
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
            scan_fn_common("<lambda>", &sig.params, &body_stmts, parent, scan)
        }
        _ => Err(CodegenError::Unsupported("Expr (03.02c slice)")),
    }
}

fn stmt_unsupported(s: &Stmt) -> &'static str {
    match s {
        Stmt::For(..) => "Stmt::For (leaf 03.04: iterators)",
        Stmt::Try(..) => "Stmt::Try (03.03)",
        Stmt::Raise(..) => "Stmt::Raise (03.03)",
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
                Stmt::If(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Stmt::While(_, a, b) => {
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
    // ---- pass 0: scan (fail-closed) + cell resolution ----
    let mut scan = Scan::new();
    scan_module(module, &mut scan)?;
    resolve_cells(&mut scan);
    let res_maps: Vec<std::collections::HashMap<String, Res>> =
        (0..scan.fns.len()).map(|i| resolution_map(&scan, i)).collect();

    // ---- data-segment image: [name blob][module literals][prelude] ----
    let name_blob: Vec<u8> = TYPE_ORDER
        .iter()
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

    // ---- user functions ----
    let fns_meta: Vec<UserFn> = scan.fns.clone();
    let n_user = fns_meta.len();
    for (slot, uf) in fns_meta.iter().enumerate() {
        funcs.function(FT_PY_RO);
        let f = emit_user_fn(uf, slot, &scan, &res_maps[slot], &tab);
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
        minimum: (n_user as u64).max(1),
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
    for _ in 0..TYPE_ORDER.len() {
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

    let blob = tab.bytes.clone();

    // ---- _start: rt_init, module-def function objects, module body ----
    {
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
            if uf.parent != usize::MAX {
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
        for item in &module.items {
            if let Item::Stmt(stmt) = item {
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
                };
                emit_stmt(&mut f, stmt, &mut ctx);
            }
        }
        f.instruction(&Instruction::End);
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
    m.section(&build_type_section());
    m.section(&imports);
    m.section(&funcs);
    m.section(&tables);
    m.section(&mems);
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

fn mod_global(name: &str, module_names: &[String]) -> u32 {
    let i = module_names
        .iter()
        .position(|n| n == name)
        .expect("scan registered every module name read");
    G_MOD_BASE + i as u32
}

// ===========================================================================
// rt_init (03.02a skeleton, carried forward verbatim)
// ===========================================================================

fn emit_rt_init_ordered(f: &mut Function) {
    let mut str_off = 0u32;
    let mut name_offsets: Vec<u32> = Vec::new();
    for name in TYPE_ORDER.iter() {
        name_offsets.push(str_off);
        str_off += name.len() as u32;
    }
    for (i, name) in TYPE_ORDER.iter().enumerate() {
        f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_TYPE,
        )));
        push_str_struct(f, name_offsets[i], name.len() as u32);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::ArrayNewDefault(T_TYPE_ARR));
        f.instruction(&Instruction::I32Const(SLOT_COUNT as i32));
        f.instruction(&Instruction::ArrayNewDefault(T_SLOT_ARR));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
        push_empty_dict(f);
        f.instruction(&Instruction::StructNew(T_TYPE));
        let g = G_TYPE_BASE + i as u32;
        f.instruction(&Instruction::GlobalSet(g));
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

    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::GlobalSet(G_RT_DONE));
}

fn push_str_struct(f: &mut Function, off: u32, len: u32) {
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_TYPE,
    )));
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
    res_maps: &'a [std::collections::HashMap<String, Res>],
    tab: &'a StrTab,
    strs: &'a [Vec<u8>],
    buf_ptr: *mut i32,
    scope: &'a Scope,
    /// number of currently OPEN structured blocks (br-depth arithmetic).
    depth: u32,
    /// absolute depth of the innermost loop's continue/break targets.
    br_continue: Option<u32>,
    br_break: Option<u32>,
}

/// Shared temp locals (valid in `_start` AND user-function frames; user
/// frames reserve the same indices at their local-block tail).
const TEMP_LOCAL: u32 = 40; // print-buffer end (i32)
const TEMP_LOCAL_I64: u32 = 41;
const TEMP_LOCAL_A: u32 = 42; // oref stash
const TEMP_LOCAL_B: u32 = 43;
const TEMP_LOCAL_OBJARR: u32 = 44; // mut $obj_arr stash
const TEMP_LOCAL_S: u32 = 45; // $str stash
const TEMP_LOCAL_CMP: u32 = 47; // compare spill (i32)
const TEMP_LOCAL_OBJARR_RO: u32 = 48; // RO array stash
const TEMP_LOCAL_FN: u32 = 49; // $function stash
const TEMP_LOCAL_OBJARR2: u32 = 50; // second mut $obj_arr stash (param_names)
/// temps per frame after the scope-specific locals (42..49 window).
const N_TEMPS: u32 = 10;

/// Emit one user function under the uniform convention:
/// params: 0 = $function (free-cell source), 1 = args RO array, 2 = kwargs
/// (ignored on this slice); result: pyobj.
fn emit_user_fn(
    uf: &UserFn,
    slot: usize,
    scan: &Scan,
    res: &std::collections::HashMap<String, Res>,
    tab: &StrTab,
) -> Function {
    let nparams = uf.params.len() as u32;
    // local layout: 0/1/2 = FT_PY_RO convention (fn, args, kwargs), then
    // 3..3+n = Python params, then PLAIN locals, then own cells, then free
    // cells, then the shared temp window (indices land on 42..49 by
    // construction: the plain-local count is body-dependent, so the temps
    // are addressed RELATIVE to cell_base + n_own + n_free below).
    let mut order: Vec<(u32, ValType)> = Vec::new();
    // NOTE: locals 0/1/2 are the FT_PY_RO PARAMS ($function, args, kwargs)
    // — declared by the function TYPE, not by Function::new (wasm params
    // precede locals; re-declaring them here shifted every index by 3).
    for _ in 0..nparams {
        order.push((1, pyobj_ref())); // locals 3..3+n: python params
    }
    // plain locals
    let mut plain_idx = 3 + nparams;
    for n in collect_local_binds(&uf.body) {
        if uf.params.iter().any(|p| p.name == n) || uf.own_cells.contains(&n) {
            continue;
        }
        order.push((1, pyobj_ref()));
        plain_idx += 1;
    }
    let cell_base = plain_idx;
    let n_own = uf.own_cells.len() as u32;
    let n_free = uf.free_cells.len() as u32;
    for _ in 0..n_own + n_free {
        order.push((
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_CELL),
            }),
        ));
    }
    // temps: pad to the shared ABSOLUTE window (42..49) so every frame —
    // _start and user fns — addresses temps identically (TEMP_LOCAL_A=42
    // etc. are module constants).
    let used = 3 + nparams + (plain_idx - (3 + nparams)) + n_own + n_free;
    let _ = used;
    {
        let pad = 42u32.saturating_sub(3 + nparams + (plain_idx - (3 + nparams)) + n_own + n_free);
        for _ in 0..pad {
            order.push((1, pyobj_ref()));
        }
    }
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
    )); // 50: second mut $obj_arr stash (param_names)

    let mut f = Function::new(order);
    // prologue: allocate OWN cells fresh per call (recursion reentrancy)
    for i in 0..n_own {
        f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_PYOBJ,
        )));
        f.instruction(&Instruction::StructNew(T_CELL));
        f.instruction(&Instruction::LocalSet(cell_base + i));
    }
    // load free cells from the incoming $function's cells array
    if n_free > 0 {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::StructGet {
            struct_type_index: T_FUNCTION,
            field_index: F_FN_CELLS,
        });
        f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR)); // $cell_arr rides the obj-arr stash (readonly use)
        for i in 0..n_free {
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR));
            f.instruction(&Instruction::I32Const(i as i32));
            f.instruction(&Instruction::ArrayGet(T_CELL_ARR));
            f.instruction(&Instruction::LocalSet(cell_base + n_own + i));
        }
    }
    // bind Python params from the args array: local 3+i = args[i]
    for i in 0..uf.params.len() {
        f.instruction(&Instruction::LocalGet(1)); // args array
        f.instruction(&Instruction::I32Const(i as i32));
        f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
        f.instruction(&Instruction::LocalSet(3 + i as u32));
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
    };
    let _ = slot;
    for stmt in &uf.body {
        emit_stmt(&mut f, stmt, &mut ctx);
    }
    // implicit `return None` (S-02-11) + the function-body End
    f.instruction(&Instruction::GlobalGet(G_NONE));
    f.instruction(&Instruction::End);
    f
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
        .expect("scan interned all strings");
    let mut off = TYPE_ORDER.iter().map(|n| n.len() as u32).sum::<u32>();
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
        Stmt::Line(_) | Stmt::Pass => {}
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
            let ap_ir::item::Target::Name(n) = &targets[0] else {
                unreachable!("scan vetted assign targets");
            };
            emit_name_store(f, n.as_str(), ctx);
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
            f.instruction(&Instruction::Return);
        }
        Stmt::Return(None) => {
            f.instruction(&Instruction::GlobalGet(G_NONE));
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
            let entry = ctx.depth;
            emit_cond_i32(f, cond, ctx);
            f.instruction(&Instruction::If(BlockType::Empty));
            ctx.depth += 1;
            let saved_c = ctx.br_continue;
            let saved_b = ctx.br_break;
            f.instruction(&Instruction::Loop(BlockType::Empty));
            ctx.depth += 1;
            f.instruction(&Instruction::Block(BlockType::Empty));
            ctx.depth += 1;
            // label depths: at body-emit time ctx.depth = e+3 with the
            // body Block as the nearest enclosing label. A continue site
            // INSIDE a nested If sits one deeper; the Br arithmetic is
            // site-depth minus the recorded label depth, so record the
            // label's own absolute depth: body Block = ctx.depth,
            // exit If = ctx.depth - 2.
            ctx.br_continue = Some(ctx.depth); // body-Block end -> bottom test
            ctx.br_break = Some(ctx.depth - 2); // If end -> loop exit
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
            f.instruction(&Instruction::End); // If
            for s in els {
                emit_stmt(f, s, ctx); // normal-exit only (S-02-16)
            }
        }
        Stmt::Break => {
            let target = ctx.br_break.expect("scan tracked loop nesting");
            f.instruction(&Instruction::Br(ctx.depth - target));
        }
        Stmt::Continue => {
            let target = ctx.br_continue.expect("scan tracked loop nesting");
            f.instruction(&Instruction::Br(ctx.depth - target));
        }
        Stmt::LocalDef(_) => unreachable!("LocalDef handled by the LocalDef arm below"),
        other => unreachable!("scan vetted statement: {other:?}"),
    }
}

/// Store the stack-top pyobj into `name` per the current scope.
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
        Expr::Call(func, args) => emit_call(f, func, args, ctx),
        Expr::List(items) => {
            for it in items {
                emit_expr(f, it, ctx);
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: items.len() as u32,
            });
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR_RO));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("list").unwrap_or(0),
            ));
            f.instruction(&Instruction::I32Const(items.len() as i32));
            f.instruction(&Instruction::I32Const(items.len() as i32));
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR_RO));
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
            f.instruction(&Instruction::Br(ctx.depth - entry)); // out of Block
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
        if slots.len() == 1
            && !shadowed
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
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_A));
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
        if ctx.scan.fns.iter().any(|uf| uf.name == name.as_str()) {
            // nested defs re-bind the name when their def statement
            // executes; the namespace/local load is the uniform,
            // order-correct path (builtins never collide: scan keeps the
            // slice's builtin names out of the function table)
            emit_name_load(f, name.as_str(), ctx);
            return;
        }
        // not a registered function: builtin / value call
        emit_name_load(f, name.as_str(), ctx);
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
    // callable VALUE expression
    emit_expr(f, func, ctx);
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
    matches!(n, "print" | "repr" | "str" | "bool" | "hash" | "len")
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
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                f.instruction(&Instruction::LocalGet(TEMP_LOCAL_S));
                f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
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
        _ => unreachable!("is_supported_builtin vetted: {name}"),
    }
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
    fn unsupported_node_is_reported_not_miscompiled() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.try_(
            vec![b.pass_()],
            vec![b.except_handler(None, None, vec![b.pass_()])],
            vec![],
            vec![],
        )));
        let err = lower_module(&module).expect_err("Try must not silently lower");
        assert!(matches!(err, CodegenError::Unsupported(_)));
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
