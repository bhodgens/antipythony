
use ap_codegen_wasm::lower_module;
use ap_ir::{Builder, Item, Module};
#[test]
fn v() {
    let b = Builder::new();
    let mut module = Module::new();
    module.items.push(Item::Stmt(b.expr_stmt(
        b.call(b.name("print"), vec![b.arg_pos(b.str_lit("hi"))]),
    )));
    let bytes = lower_module(&module).unwrap();
    let mut validator = wasmparser::Validator::new_with_features(
        wasmparser::WasmFeatures::default()
            | wasmparser::WasmFeatures::EXCEPTIONS
            | wasmparser::WasmFeatures::GC,
    );
    match validator.validate_all(&bytes) {
        Ok(_) => println!("VALID"),
        Err(e) => println!("INVALID: {e}"),
    }
    std::fs::write("/tmp/min033.wasm", &bytes).unwrap();
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_exceptions(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    match wasmtime::Module::new(&engine, &bytes) {
        Ok(_) => println!("COMPILE OK"),
        Err(e) => println!("COMPILE FAIL: {e:?}"),
    }
}
