// Validate the min-probe module and print the validator's full error.
#[test]
fn diag_validate_file() {
    let bytes = std::fs::read("/tmp/min.wasm").expect("read /tmp/min.wasm");
    use wasmparser::Validator;
    let feats = wasmparser::WasmFeatures::default()
        | wasmparser::WasmFeatures::GC
        | wasmparser::WasmFeatures::FUNCTION_REFERENCES
        | wasmparser::WasmFeatures::EXCEPTIONS;
    match Validator::new_with_features(feats).validate_all(&bytes) {
        Ok(_) => println!("MIN VALIDATES (diag)"),
        Err(e) => println!("MIN VALIDATE-ERR: {}", e),
    }
}
