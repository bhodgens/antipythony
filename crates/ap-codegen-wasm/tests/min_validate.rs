// Minimal single-compare validate probe: `print(1 == 1.0)`.
#[test]
fn min_validate() {
    let src = std::fs::read_to_string("/tmp/probe6.py").unwrap();
    let (m, _) = ap_parse::lower_source(&src).unwrap();
    let bytes = ap_codegen_wasm::lower_module(&m).unwrap();
    std::fs::write("/tmp/min.wasm", &bytes).unwrap();
    use wasmparser::Validator;
    let feats = wasmparser::WasmFeatures::default()
        | wasmparser::WasmFeatures::GC
        | wasmparser::WasmFeatures::FUNCTION_REFERENCES
        | wasmparser::WasmFeatures::EXCEPTIONS;
    match Validator::new_with_features(feats).validate_all(&bytes) {
        Ok(_) => println!("MIN VALIDATES"),
        Err(e) => println!("MIN VALIDATE-ERR: {}", e),
    }
}
