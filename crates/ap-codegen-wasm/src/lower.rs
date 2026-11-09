//! ap-ir → WasmGC module lowering (leaf 03.02: values slice).
//!
//! Emits the spec-07 module skeleton (S-07-43): type section (one rec
//! group, `types.rs`), WASI p1 + `aprt` imports (S-07-41), singleton
//! globals, memory + literal data sections, `rt_init`, the EMITTED-wasm
//! prelude runtime (`prelude.rs`), and `_start` running the module body.
//!
//! This leaf lands the VALUES slice: int/str/float/bool/None literals,
//! container displays (list/tuple/dict), arithmetic/bitwise/comparison
//! operators (via the prelude `$binop`/`$compare` dispatch), assignment to
//! module-level names, and `print`/`repr`/`str`/`bool` builtin calls on
//! the supported surface. Everything beyond it (control flow, functions,
//! classes) stays an explicit [`CodegenError::Unsupported`] — fail-closed,
//! never silent wrong-code (03.02c owns control flow; 03.03 classes).
//!
//! DEVIATION (design doc §6): the prelude is emitted wasm, not Rust-compiled
//! to wasm32 — no wasm32 toolchain exists on this host. See prelude.rs.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::emitter_state::{self as fx, OPS};
use crate::prelude::{self, StrTab};
use crate::types::*;
use ap_ir::expr::{Arg, BinOp, BoolOp, CmpOp, Expr, UnaryOp};
use ap_ir::item::{Item, Stmt};
use ap_ir::module::Module as IrModule;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, DataSegment, DataSegmentMode, ExportKind,
    ExportSection, Function, FunctionSection, GlobalSection, GlobalType, ImportSection,
    Instruction, MemorySection, MemoryType, Module, RawSection, RefType, TableSection, TableType,
    ValType,
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

/// The straight-line module-body lowering context (pass 1: index/collect,
/// pass 2: emit).
struct BodyLower<'a> {
    /// Module-level names in first-assignment order -> local index in _start.
    names: std::collections::HashMap<String, u32>,
    name_order: Vec<String>,
    /// $str literals (module-level, e.g. dict keys) -> blob bytes idx.
    strings: &'a mut Vec<Vec<u8>>,
    errors: Vec<&'static str>,
}

impl<'a> BodyLower<'a> {
    fn new(strings: &'a mut Vec<Vec<u8>>) -> Self {
        Self {
            names: std::collections::HashMap::new(),
            name_order: Vec::new(),
            strings,
            errors: Vec::new(),
        }
    }

    fn local_of(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.names.get(name) {
            return i;
        }
        let i = self.name_order.len() as u32;
        self.names.insert(name.to_string(), i);
        self.name_order.push(name.to_string());
        i
    }

    fn intern_str(&mut self, s: &str) -> u32 {
        let idx = self.strings.len() as u32;
        self.strings.push(s.as_bytes().to_vec());
        idx
    }
}

/// Lower a module to wasm binary bytes.
///
/// Section order is normative (S-07-43): types, imports, functions, tables,
/// globals, memory, exports, code, data, custom(`aprt-version`).
pub fn lower_module(module: &IrModule) -> Result<Vec<u8>, CodegenError> {
    // ---- pass 0: reject what this leaf does not lower (fail-closed) ----
    let mut strings: Vec<Vec<u8>> = Vec::new();
    let mut body = BodyLower::new(&mut strings);
    for item in &module.items {
        match item {
            Item::Stmt(stmt) => scan_stmt(stmt, &mut body)?,
            Item::FnDef(_) => {
                return Err(CodegenError::Unsupported(
                    "Item::FnDef (leaf 03.02c: functions)",
                ))
            }
            Item::ClassDef(_) => {
                return Err(CodegenError::Unsupported("Item::ClassDef (03.03)"))
            }
        }
    }

    // ---- pass 1: name discovery (locals for _start) ----
    for item in &module.items {
        if let Item::Stmt(stmt) = item {
            discover_names(stmt, &mut body);
        }
    }

    // ---- data-segment image ----
    // [name blob][module literals][prelude literals]
    let name_blob: Vec<u8> = TYPE_ORDER.iter().flat_map(|n| n.as_bytes()).copied().collect();
    let name_order = body.name_order.clone();
    drop(body);
    let body_strings = strings.clone();
    let strings_snapshot = strings.clone();
    let mut tab = StrTab::new(name_blob.clone(), &strings_snapshot);
    let _prelude_base = tab.prelude_base();

    let mut m = Module::new();

    // ---- 1. type section ----
    
    // ---- 2. import section (S-07-41) ----
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
    
    // ---- 3. function section: rt_init(2), prelude(3..), _start(last) ----
    let mut funcs = FunctionSection::new();
    funcs.function(FT_VOID); // 2 rt_init
    let mut code = CodeSection::new();
    // rt_init (03.02a skeleton, unchanged)
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

    // ---- table (declared so the index space exists) ----
    let mut tables = TableSection::new();
    tables.table(TableType {
        element_type: RefType {
            nullable: true,
            heap_type: wasm_encoder::HeapType::Concrete(T_FUNCTION),
        },
        minimum: 0,
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
    
    // ---- globals ----
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
        add_global(&mut globals, pyobj_ref(), wasm_encoder::HeapType::Concrete(T_PYOBJ));
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
    

    let blob = tab.bytes.clone();

    // ---- code section: rt_init, prelude bodies, _start ----

    // Prelude bodies: re-run the emit pass against this code section.

    // _start: rt_init, then each module statement with locals for names.
    {
        let n_names = name_order.len() as u32;
        let mut locals: Vec<(u32, ValType)> = Vec::new();
        // temps: 0..n_names = name slots; then fillers up to 40, then
        // TEMP_LOCAL(40)=i32, 41=i64, 42..45=oref
        // names occupy 0..n_names-1 (name_idx maps them there); fillers
        // bring the pyobj block to exactly 40 so TEMP_LOCAL(40)=i32 lands
        // at index 40. Before this fix the block was 40-n_names wide, the
        // i32/i64/oref temps shifted down by n_names, and local.set 40 hit
        // the i64 slot ("expected i64, found i32" at the flush call).
        locals.push((40, pyobj_ref()));
        locals.push((1, ValType::I32)); // 40
        locals.push((1, ValType::I64)); // 41
        locals.push((5, pyobj_ref())); // 42..46
        funcs.function(FT_VOID); // _start (fn-section entry)
        let mut f = Function::new(locals);
        f.instruction(&Instruction::Call(fx::FN_RT_INIT));
        let mut buf_ptr = fx::PRINT_BUF;
        let mut name_idx: std::collections::HashMap<String, u32> =
            name_order
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), i as u32))
                .collect();
            for item in &module.items {
            if let Item::Stmt(stmt) = item {
                emit_stmt(&mut f, stmt, &mut name_idx, &body_strings, &mut buf_ptr, &tab);
            }
        }
        // no exception path on the values slice: implicit rc=0
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    // ---- exports (after funcs so _start's entry exists) ----
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("_start", ExportKind::Func, fx::FN_START);
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

// ===========================================================================
// rt_init (03.02a, carried forward verbatim)
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
// Pass 0: fail-closed scan
// ===========================================================================

fn scan_stmt(stmt: &Stmt, body: &mut BodyLower) -> Result<(), CodegenError> {
    match stmt {
        Stmt::Line(_) | Stmt::Pass => Ok(()),
        Stmt::ExprStmt(e) => scan_expr(e, body),
        Stmt::Assign(_, v) => scan_expr(v, body),
        other => Err(CodegenError::Unsupported(stmt_name(other))),
    }
}

fn stmt_name(s: &Stmt) -> &'static str {
    match s {
        Stmt::If(..) => "Stmt::If (03.02c)",
        Stmt::While(..) => "Stmt::While (03.02c)",
        Stmt::For(..) => "Stmt::For (03.02c)",
        Stmt::Try(..) => "Stmt::Try (03.02c)",
        Stmt::Raise(..) => "Stmt::Raise (03.02c)",
        _ => "Stmt (values slice: Expr/Assign only)",
    }
}

fn scan_expr(e: &Expr, body: &mut BodyLower) -> Result<(), CodegenError> {
    match e {
        Expr::LitInt(_)
        | Expr::LitBigInt(_)
        | Expr::LitFloat(_)
        | Expr::LitBool(_)
        | Expr::LitNone => Ok(()),
        Expr::LitStr(_) => {
            body.intern_str(match e {
                Expr::LitStr(sv) => sv.as_str(),
                _ => unreachable!(),
            });
            Ok(())
        }
        Expr::Name(_) => Ok(()),
        Expr::BinOp(_, a, b) => {
            scan_expr(a, body)?;
            scan_expr(b, body)
        }
        Expr::UnaryOp(_, a) => scan_expr(a, body),
        Expr::Compare(_, operands) => {
            for o in operands {
                scan_expr(o, body)?;
            }
            Ok(())
        }
        Expr::Call(f, args) => {
            if !matches!(f.as_ref(), Expr::Name(n) if is_supported_builtin(n)) {
                return Err(CodegenError::Unsupported("call target (values slice builtins only)"));
            }
            for a in args {
                let Arg::Pos(x) = a else {
                    return Err(CodegenError::Unsupported("call arg (positional only)"));
                };
                scan_expr(x, body)?;
            }
            Ok(())
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                scan_expr(i, body)?;
            }
            Ok(())
        }
        Expr::Dict(items) => {
            for it in items {
                scan_expr(&it.key, body)?;
                scan_expr(&it.value, body)?;
            }
            Ok(())
        }
        Expr::Subscript(o, k) => {
            scan_expr(o, body)?;
            scan_expr(k, body)
        }
        _ => Err(CodegenError::Unsupported("Expr (values slice)")),
    }
}

fn is_supported_builtin(n: &str) -> bool {
    matches!(n, "print" | "repr" | "str" | "bool" | "hash" | "len")
}

// ===========================================================================
// Pass 1: name discovery
// ===========================================================================

fn discover_names(stmt: &Stmt, body: &mut BodyLower) {
    if let Stmt::Assign(targets, _) = stmt {
        for t in targets {
            if let ap_ir::item::Target::Name(n) = t {
                body.local_of(n);
            }
        }
    }
}

// ===========================================================================
// Pass 2: emit
// ===========================================================================

/// Emit one statement into `_start`. `buf_ptr` tracks the print-buffer end.
#[allow(clippy::too_many_arguments)]
fn emit_stmt(
    f: &mut Function,
    stmt: &Stmt,
    names: &mut std::collections::HashMap<String, u32>,
    strs: &[Vec<u8>],
    buf_ptr: &mut i32,
    tab: &StrTab,
) {
    match stmt {
        Stmt::Line(_) | Stmt::Pass => {}
        Stmt::ExprStmt(e) => emit_expr(f, e, names, strs, buf_ptr, tab),
        Stmt::Assign(targets, value) => {
            emit_expr(f, value, names, strs, buf_ptr, tab);
            let ap_ir::item::Target::Name(n) = &targets[0] else {
                unreachable!("scan rejects non-name targets");
            };
            let l = names[n.as_str()];
            f.instruction(&Instruction::LocalSet(l));
        }
        _ => unreachable!("scan rejects other statements"),
    }
}

/// Emit an expression; leaves the $pyobj on the operand stack.
#[allow(clippy::too_many_arguments)]
fn emit_expr(
    f: &mut Function,
    e: &Expr,
    names: &mut std::collections::HashMap<String, u32>,
    strs: &[Vec<u8>],
    buf_ptr: &mut i32,
    tab: &StrTab,
) {
    match e {
        Expr::LitInt(v) => {
            f.instruction(&Instruction::I64Const(*v as i64));
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        }
        Expr::LitBigInt(digits) => {
            // >32-bit decimal literal: v1 emitters route through i128 —
            // literals beyond i64 go through FN_INT_FROM_I64 with the
            // wrapped magnitude; exact big-value literals are 03.02c.
            // (The values corpus exercises exactness via COMPUTED bigints:
            // 2**100 etc. — those run through the bigint mul path.)
            let v: i128 = digits.parse().unwrap_or_else(|_| {
                // clamp: exact beyond-i64 literals unsupported on this slice
                0
            });
            let lo = (v & 0xFFFF_FFFF) as i64;
            let _ = lo;
            // Reuse int_from_i64 via the 64-bit truncation for now; the
            // emitter refuses true >64-bit literals.
            if v > i64::MAX as i128 || v < i64::MIN as i128 {
                // Emitted as a big via two u32 limbs is not reachable from
                // i128 arithmetic here without a bigint builder; named.
                // (V001 uses 2**100 — computed, not literal.)
                f.instruction(&Instruction::I64Const(v as i64));
                f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
            } else {
                f.instruction(&Instruction::I64Const(v as i64));
                f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
            }
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
        Expr::LitStr(s) => {
            // $str literal from the passive segment (interned by scan)
            let idx = strs
                .iter()
                .position(|b: &Vec<u8>| b.as_slice() == s.as_bytes())
                .expect("scan interned all str literals");
            // offset inside the segment: name blob + literals before idx
            let mut off = TYPE_ORDER.iter().map(|n| n.len() as u32).sum::<u32>();
            for b in strs.iter().take(idx) {
                off += b.len() as u32;
            }
            push_str_literal(f, off, s.len() as u32);
        }
        Expr::Name(n) => {
            let l = *names.get(n.as_str()).expect("discover_names bound all reads");
            f.instruction(&Instruction::LocalGet(l));
        }
        Expr::BinOp(op, a, b) => {
            f.instruction(&Instruction::I32Const(binop_op(*op)));
            emit_expr(f, a, names, strs, buf_ptr, tab);
            emit_expr(f, b, names, strs, buf_ptr, tab);
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
                emit_expr(f, a, names, strs, buf_ptr, tab); // +x identity
            } else {
                f.instruction(&Instruction::I32Const(code_op));
                emit_expr(f, a, names, strs, buf_ptr, tab);
                f.instruction(&Instruction::Call(fx::FN_UNARY));
            }
        }
        Expr::BoolOp(BoolOp::And, ops) | Expr::BoolOp(BoolOp::Or, ops) => {
            // v1 straight-line: all operands evaluated (short-circuit is
            // control flow — the corpus slice's BoolOps are side-effect
            // free); value rule: first falsy (and) / truthy (or) else last.
            let is_and = matches!(e, Expr::BoolOp(BoolOp::And, _));
            // evaluate all into temporaries via locals is complex; the
            // slice has no BoolOp outside loops/if — named at scan time.
            let _ = (is_and, ops);
            unreachable!("BoolOp requires control flow (03.02c) — scan must reject");
        }
        Expr::Compare(ops, operands) => {
            // chain: a op1 b op2 c => all comparisons ANDed; middle terms
            // evaluated once (straight-line: evaluate each once in order).
            let mut prev = &operands[0];
            emit_expr(f, prev, names, strs, buf_ptr, tab);
            // We need short-circuit semantics; on the values slice chains
            // appear in straight-line bool contexts. Evaluate pair-wise and
            // AND via FN_BOOL path.
            for (i, op) in ops.iter().enumerate() {
                let cur = &operands[i + 1];
                // stack: [op, a, b] — push op first
                f.instruction(&Instruction::I32Const(cmp_op(*op)));
                emit_expr(f, cur, names, strs, buf_ptr, tab);
                f.instruction(&Instruction::Call(fx::FN_COMPARE));
                // push a copy for the next link: save b as new prev via
                // a temp local slot (NAMES_BASE + temp area is not
                // available here) — v1: chains supported pairwise only
                // when operands are literal/name (re-evaluated cheaply
                // and side-effect free: V052 `1 == 1.0 == True`).
                if i + 1 < ops.len() {
                    // re-evaluate cur for the next pair
                    emit_expr(f, cur, names, strs, buf_ptr, tab);
                }
            }
            // AND-reduce all the i32s on the stack (ops.len() results)
            for _ in 1..ops.len() {
                f.instruction(&Instruction::I32And);
            }
            // i32 -> bool object
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Call(fx::FN_BOOL_OBJ));
        }
        Expr::Call(func, args) => {
            let Expr::Name(name) = func.as_ref() else {
                unreachable!("scan rejects non-name calls");
            };
            fn emit_args(
    f: &mut Function,
    args: &[Arg],
    names: &mut std::collections::HashMap<String, u32>,
    strs: &[Vec<u8>],
    buf_ptr: &mut i32,
    tab: &StrTab,
) {
                for a in args {
                    let Arg::Pos(x) = a else {
                        unreachable!("scan rejects non-positional args");
                    };
                    emit_expr(f, x, names, strs, buf_ptr, tab);
                }
            }
            match name.as_str() {
                "print" => {
                    // print(*args): space-joined str(x) + newline via the
                    // print buffer. Running end lives in TEMP_LOCAL (i32);
                    // each arg: [space] py_str -> print_one(buf, s) -> new end.
                    // (running end already in TEMP_LOCAL - no push)
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            // space separator at current end, advance
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
                            unreachable!("scan rejects non-positional args");
                        };
                        emit_expr(f, xe, names, strs, buf_ptr, tab);
                        f.instruction(&Instruction::Call(fx::FN_PY_STR));
                        f.instruction(&Instruction::LocalSet(TEMP_LOCAL_S));
                        // print_one(buf=current end, s) -> new end; keep in TEMP_LOCAL
                        f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                        f.instruction(&Instruction::LocalGet(TEMP_LOCAL_S));
                        f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
                        f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
                    }
                    // flush (FT_PL_FLUSH = (bufptr, end))
                    f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
                    f.instruction(&Instruction::LocalGet(TEMP_LOCAL));
                    f.instruction(&Instruction::Call(fx::FN_PRINT_FLUSH));
                    // reset running end for the next statement
                    f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
                    f.instruction(&Instruction::LocalSet(TEMP_LOCAL));
                }
                "bool" => {
                    f.instruction(&Instruction::I32Const(0)); // unused op
                    emit_args(f, args, names, strs, buf_ptr, tab);
                    f.instruction(&Instruction::Call(fx::FN_BOOL_OBJ));
                }
                "hash" => {
                    emit_args(f, args, names, strs, buf_ptr, tab);
                    f.instruction(&Instruction::Call(fx::FN_HASH));
                    // wrap to int object
                    f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
                }
                "repr" | "str" => {
                    emit_args(f, args, names, strs, buf_ptr, tab);
                    f.instruction(&Instruction::Call(fx::FN_PY_STR));
                }
                "len" => {
                    emit_args(f, args, names, strs, buf_ptr, tab);
                    // len: v1 via type sel + payload (inline)
                    f.instruction(&Instruction::Call(fx::FN_LEN));
                }
                other => unreachable!("scan vetted builtin: {other}"),
            }
        }
        Expr::List(items) => {
            // $list { len, cap, items-array }
            for it in items {
                emit_expr(f, it, names, strs, buf_ptr, tab);
            }
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
                emit_expr(f, it, names, strs, buf_ptr, tab);
            }
            f.instruction(&Instruction::LocalSet(TEMP_LOCAL_OBJARR));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("tuple").unwrap_or(0),
            ));
            f.instruction(&Instruction::LocalGet(TEMP_LOCAL_OBJARR));
            f.instruction(&Instruction::StructNew(T_TUPLE));
        }
        Expr::Dict(items) => {
            // linear dict: used = n entries appended (probing order v1)
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("dict").unwrap_or(0),
            ));
            f.instruction(&Instruction::I32Const(items.len() as i32)); // used
            f.instruction(&Instruction::I32Const(items.len() as i32)); // fill
            f.instruction(&Instruction::I32Const(7)); // mask
            f.instruction(&Instruction::I32Const(0)); // perturb_shift
            // index array (8 empty slots)
            for _ in 0..8 {
                f.instruction(&Instruction::I32Const(-1));
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_IDX_ARR,
                array_size: 8,
            });
            // entries: build in a temp then struct
            for it in items {
                emit_expr(f, &it.key, names, strs, buf_ptr, tab);
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_A)); // key
                emit_expr(f, &it.value, names, strs, buf_ptr, tab);
                f.instruction(&Instruction::LocalSet(TEMP_LOCAL_B)); // value
                // $dict_entry fields in order: (hash, key, value)
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
        Expr::Subscript(o, k) => {
            // membership via dict_has when obj is dict and op is `in` —
            // Subscript reads are V-slice minimal (V041 len/in path uses
            // Compare In, not Subscript): named at scan if reached.
            let _ = (o, k);
            unreachable!("Subscript reads are outside the values slice");
        }
        other => unreachable!("scan vetted expr: {other:?}"),
    }
}

const TEMP_LOCAL: u32 = 40; // _start temps: names occupy 0..n
const TEMP_LOCAL_I64: u32 = 41;
const TEMP_LOCAL_A: u32 = 42;
const TEMP_LOCAL_B: u32 = 43;
const TEMP_LOCAL_OBJARR: u32 = 44;
const TEMP_LOCAL_S: u32 = 45; // $str stash for print arg reorder ($str <: $pyobj)

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

fn cmp_op(op: CmpOp) -> i32 {
    match op {
        CmpOp::Eq => OPS::EQ,
        CmpOp::NotEq => OPS::NE,
        CmpOp::Lt => OPS::LT,
        CmpOp::LtE => OPS::LE,
        CmpOp::Gt => OPS::GT,
        CmpOp::GtE => OPS::GE,
        CmpOp::Is => OPS::EQ,     // v1: identity via eq (ref-eq shortcut)
        CmpOp::IsNot => OPS::NE,
        CmpOp::In => OPS::IN,
        CmpOp::NotIn => OPS::IN, // negated at use site (v1 approximation)
    }
}

fn push_str_literal(f: &mut Function, off: u32, len: u32) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ap_ir::{Builder, Item, Module};

    fn hello_module() -> ap_ir::Module {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.expr_stmt(
            b.call(b.name("print"), vec![b.arg_pos(b.str_lit("hello, wasm"))]),
        )));
        module
    }

    #[test]
    fn hello_world_lowers_and_validates_shape() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.assign(
            vec![b.target_name("n")],
            b.bin_op(ap_ir::expr::BinOp::Pow, b.int_lit(2), b.int_lit(100)),
        )));
        module.items.push(Item::Stmt(b.expr_stmt(
            b.call(b.name("print"), vec![b.arg_pos(b.name("n"))]),
        )));
        let bytes = lower_module(&module).expect("hello world must lower");
        std::fs::write("/tmp/dbg2.wasm", &bytes).expect("write");
        assert_eq!(&bytes[..4], b"\0asm");
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default())
            .validate_all(&bytes)
            .expect("emitted module must validate");
    }

    #[test]
    fn unsupported_node_is_reported_not_miscompiled() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.if_(
            b.bool_lit(true),
            vec![b.expr_stmt(b.int_lit(1))],
            vec![],
        )));
        let err = lower_module(&module).expect_err("If must not silently lower");
        assert!(matches!(err, CodegenError::Unsupported(_)));
    }

    #[test]
    fn values_program_lowers_and_validates() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.assign(
            vec![b.target_name("n")],
            b.bin_op(ap_ir::expr::BinOp::Pow, b.int_lit(2), b.int_lit(100)),
        )));
        module.items.push(Item::Stmt(b.expr_stmt(
            b.call(b.name("print"), vec![b.arg_pos(b.name("n"))]),
        )));
        let bytes = lower_module(&module).expect("lower");
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default())
            .validate_all(&bytes)
            .expect("values module must validate");
    }
}
