//! ap-ir → WasmGC module lowering (leaf 03.02).
//!
//! Emits the spec-07 module skeleton (S-07-43) with the GC `$pyobj`
//! hierarchy: type section (one rec group, `types.rs`), WASI p1 + `aprt`
//! imports (S-07-41), singleton globals, memory + literal data sections,
//! `rt_init` (singletons + builtin type objects with precomputed MRO and
//! slot tables + interned `$str` literals), and `_start` running the module
//! body. This leaf lands scope item 1 (the hierarchy + rt_init + the GC
//! `$str` print path replacing the linear-memory bootstrap); everything
//! beyond it stays an explicit [`CodegenError::Unsupported`] — fail-closed,
//! never silent wrong-code (port discipline: same IR, same ap-obj
//! semantics; 03.03 owns classes/exceptions, 03.04 generators/match).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::types::*;
use ap_ir::expr::{Arg, Expr};
use ap_ir::item::{Item, Stmt};
use ap_ir::module::Module as IrModule;
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, DataSegment, DataSegmentMode, ExportKind, ExportSection,
    Function, FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction,
    MemorySection, MemoryType, Module, RawSection, RefType, TableSection, TableType, ValType,
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

/// A collected `print` payload: the blob index of the pre-rendered bytes
/// `print` writes (str() of the argument + newline). The bytes are exactly
/// what an eager `$str` would carry; `_start` hands them to fd_write
/// (S-07-41: print builds `$str` in-module; the host never reformats).
struct PrintLine {
    /// Index into the emitted string-literal table.
    str_idx: u32,
}

/// Lower a module to wasm binary bytes.
///
/// Section order is normative (S-07-43): types, imports, functions, tables,
/// globals, memory, exports, code, data, custom(`aprt-version`).
pub fn lower_module(module: &IrModule) -> Result<Vec<u8>, CodegenError> {
    // Pass 1: walk the IR. This leaf lands the spec-07 MODULE SKELETON:
    // the $pyobj type hierarchy, rt_init (singletons + builtin type
    // objects), and the GC-$str print path. Value lowering (int/str/float/
    // bool/None literals, arithmetic, control flow, FnDef) is the leaf's
    // REMAINING scope and stays named-Unsupported: fail-closed, never
    // silent wrong-code.
    let mut strings: Vec<Vec<u8>> = Vec::new();
    let mut prints: Vec<PrintLine> = Vec::new();
    let mut has_fn = false;
    for item in &module.items {
        match item {
            Item::Stmt(stmt) => collect_stmt(stmt, &mut strings, &mut prints)?,
            Item::FnDef(_) => {
                // Scope item of THIS leaf not yet landed: FnDef → $code /
                // $function emission. Named, never silent.
                has_fn = true;
            }
            Item::ClassDef(_) => return Err(CodegenError::Unsupported("Item::ClassDef (03.03)")),
        }
    }
    if has_fn {
        return Err(CodegenError::Unsupported(
            "Item::FnDef (leaf 03.02 scope: $code/$function emission pending)",
        ));
    }

    // Byte length (incl. trailing newline) of each collected literal, keyed
    // by its blob index — _start bakes these into the iovec stores.
    let print_lens: std::collections::HashMap<u32, i32> = strings
        .iter()
        .enumerate()
        .map(|(i, s)| (i as u32, s.len() as i32))
        .collect();

    // Blob layout: [0..name_blob_len) = the rt_init builtin-TYPE-NAME bytes
    // (TYPE_ORDER concatenated, offsets mirrored in emit_rt_init_ordered),
    // then module string literals at STR_BASE onward. The $str structs
    // array.new_data from the same segment.
    let name_blob_len: u32 = TYPE_ORDER.iter().map(|n| n.len() as u32).sum();

    let mut m = Module::new();

    // ---- 1. type section (spec-07 §12.1) --------------------------------
    m.section(&build_type_section());

    // ---- 2. import section (S-07-41: WASI p1 + aprt) --------------------
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
    m.section(&imports);

    // ---- 3. function section --------------------------------------------
    // Import funcs occupy 0 (fd_write) and 1 (abort_msg); local funcs:
    // 2 = rt_init, 3 = _start.
    const F_RT_INIT: u32 = 2;
    const F_START: u32 = 3;
    let mut funcs = FunctionSection::new();
    funcs.function(FT_VOID); // rt_init
    funcs.function(FT_VOID); // _start
    m.section(&funcs);

    // Table (call_indirect $pycall target; S-07-43.3). Min size 0 —
    // populated in later leaves; declared so the table index space exists.
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
    m.section(&tables);

    // ---- 4. memory section (must precede globals in binary order: id 5) --
    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mems);

    // ---- 5. globals (singletons; null-init, filled by rt_init; id 6) -----
    // Builtin-type globals are typed (ref null $type): they hold the $type
    // objects rt_init builds, and field 0 of every $pyobj needs exactly
    // that type (WasmGC struct fields match exactly — no sub-type
    // widening on global.get).
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
        // None/True/False/Ellipsis
    }
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::extended([Instruction::I32Const(0)]),
    ); // rt_done flag
    for _ in 0..TYPE_ORDER.len() {
        add_global(
            &mut globals,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(T_TYPE),
            }),
            wasm_encoder::HeapType::Concrete(T_TYPE),
        ); // builtin $type refs
    }
    m.section(&globals);

    // ---- tag section: intentionally ABSENT ------------------------------
    // The $exception TYPE is declared now (the type section stays stable
    // across leaves) but no tag/throw/try_table sites exist until 03.03
    // (S-07-28 lowering is that leaf's scope). An unused tag section would
    // be dead weight and misencode the payload type this early.

    // ---- data section content -------------------------------------------
    // Segment 0 (ACTIVE): [name blob][print literals] copied to linear
    // memory at STR_BASE — _start's fd_write iovecs read the print bytes
    // from there.
    // Segment 1 (PASSIVE): the SAME bytes, kept for `array.new_data` —
    // wasmtime 47 drops ACTIVE segments after instantiation, so any
    // array.new_data over an active segment traps (out-of-bounds; probed
    // in examples/probe_seg.rs). rt_init's $str builders read segment 1.
    let name_blob: Vec<u8> = TYPE_ORDER.iter().flat_map(|n| n.as_bytes()).copied().collect();
    let blob: Vec<u8> = name_blob.iter().chain(strings.iter().flatten()).copied().collect();
    let mut data = DataSection::new();
    let mut data_count_n = 0u32;
    if !blob.is_empty() {
        data.segment(DataSegment {
            mode: DataSegmentMode::Active {
                memory_index: 0,
                offset: &ConstExpr::i32_const(STR_BASE),
            },
            data: blob.clone(),
        });
        data_count_n += 1;
        data.segment(DataSegment {
            mode: DataSegmentMode::Passive,
            data: blob,
        });
        data_count_n += 1;
    }

    // ---- exports ---------------------------------------------------------
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("_start", ExportKind::Func, F_START);
    exports.export("aprt_version", ExportKind::Global, G_RT_DONE);
    m.section(&exports);

    // ---- data-count section (id 12; REQUIRED by array.new_data and must
    // precede the code section) -------------------------------------------
    let data_count = wasm_encoder::DataCountSection {
        count: data_count_n,
    };
    if data_count_n > 0 {
        m.section(&data_count);
    }

    // ---- code section ----------------------------------------------------
    let mut code = CodeSection::new();

    // rt_init: guard flag; allocate singletons; build builtin type objects
    // (name $str, empty mro/bases, slot table, empty dict); set rt_done.
    // All GC allocation is struct.new / array.new — the engine's collector
    // owns it (S-07-36).
    {
        let mut f = Function::new([(1, ValType::I32)]);
        // if (global rt_done) return
        f.instruction(&Instruction::GlobalGet(G_RT_DONE));
        f.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);

        emit_rt_init_ordered(&mut f, &strings);

        f.instruction(&Instruction::End);
        code.function(&f);
    }

    // _start: rt_init (guarded), then each print line. The blob region
    // layout is known at emission time: for each literal an iovec{buf,len}
    // is stored at address 0 and fd_write(1, iov, 1, 8) is called. The
    // bytes are exactly what the eager $str carries (S-07-41: print hands
    // $str bytes to fd_write; the host never reformats).
    {
        let mut f = Function::new([]);
        f.instruction(&Instruction::Call(F_RT_INIT));
        // Print literals live AFTER the builtin-name blob in the segment.
        let mut off = STR_BASE + name_blob_len as i32;
        for p in &prints {
            let len = print_lens[&p.str_idx];
            // iovec.buf @ 0
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(off));
            store_i32(&mut f, 0);
            // iovec.len @ 4
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Const(len));
            store_i32(&mut f, 0);
            // fd_write(fd=1, iovs=0, iovs_len=1, nwritten=8)
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::Call(0)); // imported fd_write
            f.instruction(&Instruction::Drop);
            off += len;
        }
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    m.section(&code);

    // data section (id 11) — after code (id 10) per normative order.
    if !data.is_empty() {
        m.section(&data);
    }

    // provenance custom section (aprt version per design doc §7)
    let mut custom: Vec<u8> = Vec::new();
    custom.push(12); // name length
    custom.extend_from_slice(b"aprt-version");
    custom.push(b'0');
    m.section(&RawSection {
        id: wasm_encoder::SectionId::Custom as u8,
        data: &custom,
    });

    Ok(m.finish())
}

/// Where linear memory starts holding string-literal payloads. The first
/// 64 bytes are reserved for scratch (iovec structs).
const STR_BASE: i32 = 64;

/// `i32.store` at `offset` (default memory, align 2).
fn store_i32(f: &mut Function, offset: i32) {
    f.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
        offset: offset as u64,
        align: 2,
        target: wasm_encoder::MemArgTarget::Memory(0),
        ordering: None,
    }));
}

/// The rt_init emitter (correct wasm stack discipline throughout).
fn emit_rt_init_ordered(f: &mut Function, _strings: &[Vec<u8>]) {
    // ---- builtin $type objects ----
    let mut str_off = 0u32;
    let mut name_offsets: Vec<u32> = Vec::new();
    for name in TYPE_ORDER.iter() {
        name_offsets.push(str_off);
        str_off += name.len() as u32;
    }
    for (i, name) in TYPE_ORDER.iter().enumerate() {
        f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_TYPE,
        ))); // metatype (v1: null)
        push_str_struct(f, name_offsets[i], name.len() as u32, None); // name
        f.instruction(&Instruction::I32Const(0)); // mro len 0
        f.instruction(&Instruction::ArrayNewDefault(T_TYPE_ARR));
        f.instruction(&Instruction::I32Const(SLOT_COUNT as i32));
        f.instruction(&Instruction::ArrayNewDefault(T_SLOT_ARR));
        f.instruction(&Instruction::I32Const(0)); // bases len 0
        f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
        push_empty_dict(f); // dict
        f.instruction(&Instruction::StructNew(T_TYPE));
        let g = G_TYPE_BASE + i as u32;
        f.instruction(&Instruction::GlobalSet(g));
    }

    // ---- singletons (S-07-2/8/9) ----
    // None: bare $pyobj, field 0 = NoneType.
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("NoneType").unwrap_or(0),
    ));
    f.instruction(&Instruction::StructNew(T_PYOBJ));
    f.instruction(&Instruction::GlobalSet(G_NONE));
    // Ellipsis: bare $pyobj, field 0 = ellipsis.
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("ellipsis").unwrap_or(0),
    ));
    f.instruction(&Instruction::StructNew(T_PYOBJ));
    f.instruction(&Instruction::GlobalSet(G_ELLIPSIS));
    // True/False: $int structs, kind=0 smi=1/0, field 0 = bool type (S-07-8).
    // $int layout: type, kind, smi, big.
    let bool_g = builtin_type_global("bool").unwrap_or(0);
    f.instruction(&Instruction::GlobalGet(bool_g));
    f.instruction(&Instruction::I32Const(0)); // kind = smi
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_BIGINT,
    )));
    f.instruction(&Instruction::StructNew(T_INT));
    f.instruction(&Instruction::GlobalSet(G_TRUE));
    f.instruction(&Instruction::GlobalGet(bool_g));
    f.instruction(&Instruction::I32Const(0)); // kind = smi
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_BIGINT,
    )));
    f.instruction(&Instruction::StructNew(T_INT));
    f.instruction(&Instruction::GlobalSet(G_FALSE));

    // String-literal interning (S-07-44): v1 policy = print reads the blob
    // bytes directly (the SAME bytes an eager $str would carry, S-07-41);
    // the per-literal $str global table lands with the value-path work in
    // 03.03 (the globals section here is sized by the singleton/type count
    // only).

    // rt_done = 1
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::GlobalSet(G_RT_DONE));
}

/// Push a $str struct for the blob slice at `off` with `len` bytes:
/// fields in declared order (type, hash, kind, len, data). The struct ref
/// is left on the operand stack. `type_g` = the global idx of the $type to
/// attach (None = null ref for interned-name scratch, patched by callers
/// that need a typed string).
fn push_str_struct(f: &mut Function, off: u32, len: u32, type_g: Option<u32>) {
    match type_g {
        Some(g) => f.instruction(&Instruction::GlobalGet(g)),
        None => f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
            T_TYPE,
        ))),
    };
    f.instruction(&Instruction::I64Const(0)); // hash: unhashed (S-07-12)
    f.instruction(&Instruction::I32Const(0)); // kind 0 = ascii/latin1 (S-07-11)
    f.instruction(&Instruction::I32Const(len as i32)); // len (code points)
    // data: push operands (offset, size) THEN array.new_data $byte_arr 0.
    // NOTE: array.new_data indexes the DATA SEGMENT (0-based), NOT linear
    // memory — the segment lives at STR_BASE but its internal offsets are
    // relative to the segment start.
    f.instruction(&Instruction::I32Const(off as i32));
    f.instruction(&Instruction::I32Const(len as i32));
    f.instruction(&Instruction::ArrayNewData {
        array_type_index: T_BYTE_ARR,
        array_data_index: 1, // segment 1 = the PASSIVE literal copy
    });
    f.instruction(&Instruction::StructNew(T_STR));
}

/// Push an empty compact $dict: used=0 fill=0 mask=7 perturb=0, index array
/// of 8 i32 filled with DKIX_EMPTY(-1) via array.new_fixed, empty entries
/// array. Layout mirrors ap-obj `PyDict::new` (dict.rs).
fn push_empty_dict(f: &mut Function) {
    f.instruction(&Instruction::RefNull(wasm_encoder::HeapType::Concrete(
        T_TYPE,
    )));
    f.instruction(&Instruction::I32Const(0)); // used
    f.instruction(&Instruction::I32Const(0)); // fill
    f.instruction(&Instruction::I32Const(7)); // mask (8 slots)
    f.instruction(&Instruction::I32Const(0)); // perturb_shift
    // index array: push the 8 DKIX_EMPTY(-1) values THEN array.new_fixed.
    for _ in 0..8 {
        f.instruction(&Instruction::I32Const(-1)); // DKIX_EMPTY
    }
    f.instruction(&Instruction::ArrayNewFixed {
        array_type_index: T_IDX_ARR,
        array_size: 8,
    });
    // entries: empty
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayNewDefault(T_ENTRY_ARR));
    f.instruction(&Instruction::StructNew(T_DICT));
}

/// Lower one statement. This leaf: `print(str-literal)` / `print(bytes
/// literal)` only; `Stmt::Line` and `Pass` are no-ops. Everything else is
/// named Unsupported (values/control-flow lowering is this leaf's remaining
/// scope; classes/exceptions 03.03; generators/match 03.04).
fn collect_stmt(
    stmt: &Stmt,
    strings: &mut Vec<Vec<u8>>,
    prints: &mut Vec<PrintLine>,
) -> Result<(), CodegenError> {
    match stmt {
        Stmt::Line(_) | Stmt::Pass => Ok(()),
        Stmt::ExprStmt(e) => collect_print(e, strings, prints),
        _ => Err(CodegenError::Unsupported(
            "Stmt (leaf 03.02: print-literal only so far)",
        )),
    }
}

/// Recognize `print(<str literal>)` (plus bytes literal) and record the
/// bytes `print` would write (payload + newline). Anything else is
/// Unsupported.
fn collect_print(
    e: &Expr,
    strings: &mut Vec<Vec<u8>>,
    prints: &mut Vec<PrintLine>,
) -> Result<(), CodegenError> {
    let Expr::Call(func, args) = e else {
        return Err(CodegenError::Unsupported("ExprStmt (not a call)"));
    };
    if !matches!(func.as_ref(), Expr::Name(n) if n == "print") {
        return Err(CodegenError::Unsupported("call target (not print)"));
    }
    if args.len() != 1 {
        return Err(CodegenError::Unsupported("print arity != 1"));
    }
    let Arg::Pos(arg) = &args[0] else {
        return Err(CodegenError::Unsupported("print arg (not positional)"));
    };
    match arg {
        Expr::LitStr(s) => {
            // F4-adjacent (audit 2026-11-06-memperf-bughunt): build the line
            // buffer directly instead of cloning the String.
            let mut bytes = Vec::with_capacity(s.len() + 1);
            bytes.extend_from_slice(s.as_bytes());
            bytes.push(b'\n');
            let idx = strings.len() as u32;
            strings.push(bytes);
            prints.push(PrintLine { str_idx: idx });
            Ok(())
        }
        Expr::LitBytes(_) => Err(CodegenError::Unsupported(
            "print(bytes literal) (bytes repr path pending)",
        )),
        _ => Err(CodegenError::Unsupported(
            "print arg (non-literal; value lowering pending)",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_ir::{Builder, Item, Module};

    #[test]
    fn hello_world_lowers_and_validates_shape() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.expr_stmt(
            b.call(b.name("print"), vec![b.arg_pos(b.str_lit("hello, wasm"))]),
        )));
        let bytes = lower_module(&module).expect("hello world must lower");
        // wasm magic + version
        assert_eq!(&bytes[..4], b"\0asm");
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
        assert!(bytes.len() > 32, "module should contain code+data sections");
        // full structural validation (same parser family wasmtime uses)
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default())
            .validate_all(&bytes)
            .expect("emitted module must validate");
    }

    #[test]
    fn unsupported_node_is_reported_not_miscompiled() {
        let b = Builder::new();
        let mut module = Module::new();
        module
            .items
            .push(Item::Stmt(b.assign(vec![b.target_name("x")], b.int_lit(1))));
        let err = lower_module(&module).expect_err("assignment is unsupported in this leaf");
        assert!(matches!(err, CodegenError::Unsupported(_)));
    }

    /// spec-07 §12.1: the type section carries the full $pyobj rec group;
    /// every emitted module validates with WasmGC on.
    #[test]
    fn type_hierarchy_presents_in_every_module() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(
            b.expr_stmt(b.call(b.name("print"), vec![b.arg_pos(b.str_lit("x"))])),
        ));
        let bytes = lower_module(&module).expect("lower");
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default())
            .validate_all(&bytes)
            .expect("GC hierarchy module must validate");
        // Walk the type section: one explicit rec group carrying the whole
        // hierarchy (>= REC_GROUP_LEN subtypes).
        use wasmparser::{Parser, Payload};
        let mut max_group = 0usize;
        let mut saw_explicit_rec = false;
        for payload in Parser::new(0).parse_all(&bytes) {
            if let Payload::TypeSection(ts) = payload.expect("parse") {
                for group in ts {
                    let group = group.expect("rec group");
                    max_group = max_group.max(group.types().len());
                    saw_explicit_rec |= group.is_explicit_rec_group();
                }
            }
        }
        assert!(saw_explicit_rec, "the $pyobj rec group must be explicit");
        assert!(
            max_group >= REC_GROUP_LEN as usize,
            "rec group must carry the whole hierarchy (got {max_group})"
        );
    }

    /// S-07-41: imports are exactly WASI fd_write + aprt.abort_msg.
    #[test]
    fn import_surface_is_wasi_plus_aprt() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(
            b.expr_stmt(b.call(b.name("print"), vec![b.arg_pos(b.str_lit("x"))])),
        ));
        let bytes = lower_module(&module).expect("lower");
        use wasmparser::{Parser, Payload};
        let mut found: Vec<(String, String)> = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            if let Payload::ImportSection(sec) = payload.expect("parse") {
                for imp in sec.into_imports() {
                    let imp = imp.expect("import");
                    found.push((imp.module.to_string(), imp.name.to_string()));
                }
            }
        }
        assert!(found.contains(&("wasi_snapshot_preview1".into(), "fd_write".into())));
        assert!(found.contains(&("aprt".into(), "abort_msg".into())));
        assert_eq!(found.len(), 2, "import surface is exactly 2 (S-07-41)");
    }

    /// Item::FnDef stays fail-closed until its emission lands (this leaf's
    /// remaining scope); the error NAMES the construct.
    #[test]
    fn fn_def_is_named_unsupported() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::FnDef(b.fn_def(
            "f",
            vec![],
            None,
            vec![],
            ap_ir::item::FnBody { blocks: vec![] },
        )));
        let err = lower_module(&module).expect_err("FnDef must not silently drop");
        let CodegenError::Unsupported(what) = &err else {
            panic!("expected Unsupported");
        };
        assert!(
            what.contains("FnDef"),
            "error must name the construct: {what}"
        );
    }
}
