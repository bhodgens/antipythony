// Minimal validate probe: writes /tmp/min.wasm from an inline hello module.
#[test]
fn min_validate() {
    let b = ap_ir::Builder::new();
    let mut m = ap_ir::Module::new();
    m.items.push(ap_ir::Item::Stmt(b.expr_stmt(
        b.call(b.name("print"), vec![b.arg_pos(b.str_lit("hello, wasm"))]),
    )));
    let bytes = ap_codegen_wasm::lower_module(&m).unwrap();
    std::fs::write("/tmp/min.wasm", &bytes).unwrap();
    use wasmparser::Validator;
    let feats = wasmparser::WasmFeatures::default()
        | wasmparser::WasmFeatures::GC
        | wasmparser::WasmFeatures::FUNCTION_REFERENCES
        | wasmparser::WasmFeatures::EXCEPTIONS;
    match Validator::new_with_features(feats).validate_all(&bytes) {
        Ok(_) => println!("MIN VALIDATES"),
        Err(e) => println!("MIN VALIDATE-ERR: {} @{}", e, {
            // the binary offset within the code section is opaque; just print
            String::new()
        }),
    }
}
