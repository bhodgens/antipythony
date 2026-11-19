// Leaf 03.06 wasi-threads feasibility spike (PROBE, NOT a gate).
// VERDICT (recorded in NEXT-0306.md):
// 1. Config::wasm_threads(true) EXISTS in wasmtime 47 and a shared-memory
//    module COMPILES with it — core wasm threads is fully available.
// 2. wasmtime-wasi 47 ships NO wasi_thread_spawn host binding (grep over the
//    crate source = 0 matches) — spawning would be OUR embedding code
//    (Linker::func_wrap + std::thread::spawn handing each instance its tid).
// 3. Our production host_config refuses shared-memory modules BEFORE compile
//    (HostError::Threading, leaf 03.05 surface check) — unchanged.
// GOTCHA found en route: wasmtime 47 refuses modules with a non-default
// page_size_log2 unless custom-page-sizes is enabled — a toy probe with
// page_size_log2 Some(16) fails "failed to parse" while wasmparser validates.
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
use wasm_encoder::{
    ExportKind, ExportSection, FunctionSection, ImportSection, MemorySection, MemoryType, Module,
    RawSection, SectionId, TypeSection, ValType,
};

fn shared_mem_module() -> Vec<u8> {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], []); // type 0: () -> ()
    types.ty().function([ValType::I32], [ValType::I32]); // type 1: (i32) -> i32
    m.section(&types);
    let mut imp = ImportSection::new();
    imp.import(
        "wasi_snapshot_preview1",
        "wasi_thread_spawn",
        wasm_encoder::EntityType::Function(1),
    );
    m.section(&imp);
    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared: true,
        page_size_log2: None,
    });
    let mut funcs = FunctionSection::new();
    funcs.function(0); // _start: type 0
    let mut exports = ExportSection::new();
    exports.export("_start", ExportKind::Func, 1); // fn idx: 0=import, 1=this
    let mut code = wasm_encoder::CodeSection::new();
    let mut f = wasm_encoder::Function::new([]);
    f.instruction(&wasm_encoder::Instruction::I32Const(0));
    f.instruction(&wasm_encoder::Instruction::Drop);
    f.instruction(&wasm_encoder::Instruction::End);
    code.function(&f);
    // Section order (spec §5.5): 1 type, 2 import, 3 function, 5 memory,
    // 7 export, 10 code, then the trailing custom stamp.
    m.section(&funcs);
    m.section(&mems);
    m.section(&exports);
    m.section(&code);
    let custom: Vec<u8> = {
        let mut v = Vec::new();
        v.push(12);
        v.extend_from_slice(b"aprt-version");
        v.push(b'0');
        v
    };
    m.section(&RawSection {
        id: SectionId::Custom as u8,
        data: &custom,
    });
    let out = m.finish();
    if let Ok(dir) = std::env::var("SPIKE_DUMP") {
        let _ = std::fs::write(dir, &out);
    }
    out
}


fn shared_mem_only_module() -> Vec<u8> {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], []);
    m.section(&types);
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    m.section(&funcs);
    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared: true,
        page_size_log2: None,
    });
    m.section(&mems);
    let mut exports = ExportSection::new();
    exports.export("_start", ExportKind::Func, 0);
    m.section(&exports);
    let mut code = wasm_encoder::CodeSection::new();
    let mut f = wasm_encoder::Function::new([]);
    f.instruction(&wasm_encoder::Instruction::End);
    code.function(&f);
    m.section(&code);
    m.finish()
}

fn plain_mem_thread_import_module() -> Vec<u8> {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], []);
    types.ty().function([ValType::I32], [ValType::I32]);
    m.section(&types);
    let mut imp = ImportSection::new();
    imp.import(
        "wasi_snapshot_preview1",
        "wasi_thread_spawn",
        wasm_encoder::EntityType::Function(1),
    );
    m.section(&imp);
    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    m.section(&funcs);
    m.section(&mems);
    let mut exports = ExportSection::new();
    exports.export("_start", ExportKind::Func, 1);
    m.section(&exports);
    let mut code = wasm_encoder::CodeSection::new();
    let mut f = wasm_encoder::Function::new([]);
    f.instruction(&wasm_encoder::Instruction::End);
    code.function(&f);
    m.section(&code);
    m.finish()
}

#[test]
fn wasi_threads_spike() {
    let bytes = shared_mem_module();

    for (lbl, b) in [
        ("shared_mem_module", shared_mem_module()),
        ("shared_mem_only_module", shared_mem_only_module()),
        ("plain_mem_thread_import_module", plain_mem_thread_import_module()),
    ] {
        match wasmprinter::print_bytes(&b) {
            Ok(wat) => println!("--- {lbl} WAT ---\n{wat}\n--- end ---"),
            Err(e) => println!("--- {lbl} PRINTER-ERR: {e}"),
        }
    }

    // SPIKE-1/2: wasm_threads(true) engine + shared-memory module compile.
    let mut cfg = wasmtime::Config::new();
    cfg.wasm_threads(true);
    cfg.wasm_gc(true);
    cfg.wasm_exceptions(true);
    let engine = match wasmtime::Engine::new(&cfg) {
        Ok(e) => {
            println!(
                "SPIKE-1: Engine::new with wasm_threads(true) = OK (flag exists in wasmtime 47)"
            );
            e
        }
        Err(e) => {
            println!("SPIKE-1 FAIL: {e}");
            return;
        }
    };

    // CONTROL: a REAL corpus module (hello) parses on the same API — proves the
    // failure is in MY minimal builder, not the engine.
    let hello = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("logs/wasm-corpus/hello.wasm");
    if let Ok(hb) = std::fs::read(&hello) {
        match wasmtime::Module::new(&engine, &hb) {
            Ok(_) => println!("CONTROL: hello.wasm COMPILES on wasm_threads(true) engine"),
            Err(e) => println!("CONTROL: hello.wasm refused: {e}"),
        }
    }


    // SPIKE-0: bisect the parse failure — variants:
    for (label, variant) in [
        ("full (shared mem + thread_spawn import)", shared_mem_module()),
        ("shared mem only", shared_mem_only_module()),
        ("plain mem + thread_spawn import", plain_mem_thread_import_module()),
    ] {
        match wasmtime::Module::new(&engine, &variant) {
            Ok(_) => println!("SPIKE-0: [{label}] COMPILES on wasm_threads(true) engine"),
            Err(e) => println!("SPIKE-0: [{label}] refused: {e}"),
        }
        let mut plain_cfg = wasmtime::Config::new();
        plain_cfg.wasm_gc(true);
        plain_cfg.wasm_exceptions(true);
        let plain_engine = wasmtime::Engine::new(&plain_cfg).unwrap();
        match wasmtime::Module::new(&plain_engine, &variant) {
            Ok(_) => println!("SPIKE-0p: [{label}] COMPILES on PLAIN engine too"),
            Err(e) => println!("SPIKE-0p: [{label}] refused on PLAIN engine: {e}"),
        }
    }

    match wasmtime::Module::new(&engine, &bytes) {
        Ok(_) => println!(
            "SPIKE-2: shared-memory module COMPILED with wasm_threads(true) — core wasm threads is available in wasmtime 47"
        ),
        Err(e) => {
            println!("SPIKE-2 FAIL: shared-memory module refused: {e}");
            // Detail: wasmparser validation with threads ON for a precise reason.
            let features =
                wasmparser::WasmFeatures::from_bits_truncate(wasmparser::WasmFeatures::all().bits())
                    | wasmparser::WasmFeatures::THREADS;
            let mut v = wasmparser::Validator::new_with_features(features);
            match v.validate_all(&bytes) {
                Ok(_) => println!("SPIKE-2b: wasmparser (threads on) VALIDATES the module — the refusal is a wasmtime feature-flag/config matter"),
                Err(err) => println!("SPIKE-2b: wasmparser (threads on) REJECTS: {err}"),
            }
        }
    }

    // SPIKE-3: instantiate against wasmtime-wasi — wasi_thread_spawn has NO
    // host binding in wasmtime-wasi 47 (grep over the crate source = 0
    // matches). Instantiation with our Linker must fail on the unknown
    // import unless WE provide it.
    match ap_host::run(&bytes, 64 * 1024) {
        Err(ap_host::HostError::Threading(msg)) => println!(
            "SPIKE-3: production host_config (threads off) refuses the module BEFORE compile: {msg}"
        ),
        other => println!("SPIKE-3 UNEXPECTED: {other:?}"),
    }

    println!(
        "SPIKE-4: spawning would require OUR embedding code: Linker::func_wrap(\"wasi_snapshot_preview1\", \
         \"wasi_thread_spawn\") + std::thread::spawn handing each new instance its tid — i.e. building \
         wasi-threads ourselves on top of core wasm threads (wasmtime-wasi 47 provides nothing). \
         This is exactly the design-§9 note; recorded as the 03.06 spike verdict."
    );
}
