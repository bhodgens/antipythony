// Validate /tmp/GE004.wasm and print the function-local error context.
#[ignore = "developer scratch tool: hard-reads a /tmp/*.wasm no CI step writes"]
#[test]
fn val_diag() {
    let bytes = std::fs::read("/tmp/GE004.wasm").unwrap();
    use wasmparser::Validator;
    let feats = wasmparser::WasmFeatures::default()
        | wasmparser::WasmFeatures::GC
        | wasmparser::WasmFeatures::FUNCTION_REFERENCES
        | wasmparser::WasmFeatures::EXCEPTIONS;
    if let Err(e) = Validator::new_with_features(feats).validate_all(&bytes) {
        println!("VALIDATE-ERR {e}");
        println!("offset = 0x{:x}", e.offset());
        return;
    }
    println!("VALIDATES OK");
}
