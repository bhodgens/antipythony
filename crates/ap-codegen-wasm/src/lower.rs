//! Skeleton lowering: ap-ir [`Module`] → wasm binary bytes.
//!
//! Scope (Plan 03 hello-world skeleton, this wave is design-only):
//! module-level `print("<literal>")` statements lower to a WASI
//! `fd_write(1, iovec, 1, 0)` call over literal bytes placed in a data
//! segment (with the trailing newline `print` adds). Every other IR shape
//! returns [`CodegenError::Unsupported`] naming the node — the skeleton
//! must never silently miscompile (port discipline: same IR, same ap-obj
//! semantics; the GC struct hierarchy per
//! `docs/03-wasm-representation-design.md` lands in leaves 03.01/03.02).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use ap_ir::expr::{Arg, Expr};
use ap_ir::item::{Item, Stmt};
use ap_ir::module::Module as IrModule;
use wasm_encoder::{
    ConstExpr, DataSection, DataSegment, DataSegmentMode, ExportKind, ExportSection, Function,
    FunctionSection, ImportSection, Instruction, MemorySection, MemoryType, Module, RawSection,
    TypeSection, ValType,
};

/// Where linear memory starts holding string-literal payloads. The first
/// 64 bytes are reserved for scratch (iovec structs).
const DATA_BASE: i32 = 64;

/// Codegen failure. Skeleton stage: anything but `print(str-literal)`
/// statements is unsupported; later leaves replace these arms with real
/// lowering (03.02 values/ops, 03.03 classes/exceptions, 03.04 generators).
#[derive(Debug, PartialEq, Eq)]
pub enum CodegenError {
    /// The IR contains a node this skeleton does not lower yet.
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

/// Lower a module to wasm binary bytes.
///
/// Emits: type section (`fd_write` import type + `_start` type), the
/// `wasi_snapshot_preview1.fd_write` import, one page of memory, the
/// `_start` function running the module body in source order, and the
/// string-literal data segment.
pub fn lower_module(module: &IrModule) -> Result<Vec<u8>, CodegenError> {
    // Pass 1: collect printable payloads; bail on anything else.
    let mut payloads: Vec<Vec<u8>> = Vec::new();
    for item in &module.items {
        match item {
            Item::Stmt(stmt) => collect_stmt(stmt, &mut payloads)?,
            Item::FnDef(_) => return Err(CodegenError::Unsupported("Item::FnDef")),
            Item::ClassDef(_) => return Err(CodegenError::Unsupported("Item::ClassDef")),
        }
    }

    // Pass 2: emit.
    let mut m = Module::new();

    // types: 0 = fd_write(i32,i32,i32,i32)->i32 ; 1 = _start()->()
    let mut types = TypeSection::new();
    types.ty().function(
        [ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        [ValType::I32],
    );
    types.ty().function([], []);
    m.section(&types);

    let mut imports = ImportSection::new();
    imports.import(
        "wasi_snapshot_preview1",
        "fd_write",
        wasm_encoder::EntityType::Function(0),
    );
    m.section(&imports);

    let mut funcs = FunctionSection::new();
    funcs.function(1); // _start
    m.section(&funcs);

    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mems);

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    // func 0 is the imported fd_write; the local _start is func 1.
    exports.export("_start", ExportKind::Func, 1);
    m.section(&exports);

    // data segment: concatenated payloads at DATA_BASE. NOTE: section order
    // is normative (code = id 10, data = id 11), so the data section is
    // appended after the code section below.
    let blob: Vec<u8> = payloads.concat();

    // _start: for each payload, write an iovec at 0..8 and fd_write it.
    let iovec_store = |f: &mut Function, addr: i32| {
        f.instruction(&Instruction::I32Store(wasm_encoder::MemArg {
            offset: addr as u64,
            align: 2,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
    };
    let mut code = wasm_encoder::CodeSection::new();
    let mut f = Function::new([]);
    let mut off = DATA_BASE;
    for p in &payloads {
        // iovec { buf: off, len: p.len() } at address 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Const(off));
        iovec_store(&mut f, 0); // iov.buf  @ 0
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Const(p.len() as i32));
        iovec_store(&mut f, 0); // iov.len  @ 4
                                // fd_write(fd=1, iovs=0, iovs_len=1, nwritten=8)
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::Call(0));
        f.instruction(&Instruction::Drop);
        off += p.len() as i32;
    }
    f.instruction(&Instruction::End);
    code.function(&f);
    m.section(&code);

    if !blob.is_empty() {
        let mut data = DataSection::new();
        data.segment(DataSegment {
            mode: DataSegmentMode::Active {
                memory_index: 0,
                offset: &ConstExpr::i32_const(DATA_BASE),
            },
            data: blob,
        });
        m.section(&data);
    }

    // provenance custom section (aprt version per design doc §7).
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

/// Lower one statement. Skeleton: `print("lit")` only; `Stmt::Line` and
/// `Pass` are no-ops (line markers are traceback metadata, 03.03).
fn collect_stmt(stmt: &Stmt, out: &mut Vec<Vec<u8>>) -> Result<(), CodegenError> {
    match stmt {
        Stmt::Line(_) | Stmt::Pass => Ok(()),
        Stmt::ExprStmt(e) => collect_print(e, out),
        _ => Err(CodegenError::Unsupported(
            "Stmt (skeleton: print-literal only)",
        )),
    }
}

/// Recognize `print(<str literal>)` and push the bytes `print` would write
/// (payload + newline). Anything else is Unsupported for now.
fn collect_print(e: &Expr, out: &mut Vec<Vec<u8>>) -> Result<(), CodegenError> {
    let Expr::Call(func, args) = e else {
        return Err(CodegenError::Unsupported("ExprStmt (not a call)"));
    };
    if !matches!(func.as_ref(), Expr::Name(n) if n == "print") {
        return Err(CodegenError::Unsupported("call target (not print)"));
    }
    if args.len() != 1 {
        return Err(CodegenError::Unsupported("print arity != 1"));
    }
    let Arg::Pos(Expr::LitStr(s)) = &args[0] else {
        return Err(CodegenError::Unsupported("print arg (not str literal)"));
    };
    let mut bytes = s.clone().into_bytes();
    bytes.push(b'\n');
    out.push(bytes);
    Ok(())
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
        let err = lower_module(&module).expect_err("assignment is unsupported in skeleton");
        assert!(matches!(err, CodegenError::Unsupported(_)));
    }
}
