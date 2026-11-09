//! diag
fn main() {
    let bytes = std::fs::read("/tmp/dbg2.wasm").unwrap();
    let features = wasmparser::WasmFeatures::default()
        | wasmparser::WasmFeatures::GC
        | wasmparser::WasmFeatures::FUNCTION_REFERENCES
        | wasmparser::WasmFeatures::EXCEPTIONS;
    let mut validator = wasmparser::Validator::new_with_features(features);
    match validator.validate_all(&bytes) {
        Ok(_) => println!("wasmparser: OK"),
        Err(e) => println!("wasmparser FAIL @{}: {}", e.offset(), e),
    }
}
