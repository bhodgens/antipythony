// Validate a dumped module (DIAG_SRC=/tmp/min.wasm) with wasmparser — the
// 03.04c validator probe (min_validate pattern, but on the dumped file).
#[test]
fn dump_validate() {
    let path = std::env::var("DIAG_SRC").unwrap_or("/tmp/min.wasm".into());
    let bytes = std::fs::read(&path).unwrap();
    use wasmparser::Validator;
    let feats = wasmparser::WasmFeatures::default()
        | wasmparser::WasmFeatures::GC
        | wasmparser::WasmFeatures::FUNCTION_REFERENCES
        | wasmparser::WasmFeatures::EXCEPTIONS;
    match Validator::new_with_features(feats).validate_all(&bytes) {
        Ok(_) => println!("DUMP VALIDATES"),
        Err(e) => println!("DUMP VALIDATE-ERR: {}", e),
    }
}
