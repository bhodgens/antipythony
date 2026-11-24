// Validate the module; attribute the error to the containing function.
#[test]
fn diag_fvalidate() {
    let bytes = std::fs::read("/tmp/min.wasm").unwrap();
    use wasmparser::{Parser, Payload, WasmFeatures};
    let mut validator = wasmparser::Validator::new_with_features(
        WasmFeatures::default()
            | WasmFeatures::GC
            | WasmFeatures::FUNCTION_REFERENCES
            | WasmFeatures::EXCEPTIONS,
    );
    if let Err(e) = validator.validate_all(&bytes) {
        let off = e.offset(); // hmm may not exist; fallback below
        eprintln!("FULL-ERR: {} (offset {})", e, off);
    }
    // map offsets to functions
    let mut func_idx: u32 = 0;
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::CodeSectionEntry(body) => {
                eprintln!(
                    "code {} (fn {}) bytes {:?}",
                    func_idx,
                    func_idx + 2,
                    body.range()
                );
                func_idx += 1;
            }
            Payload::End(_) => break,
            _ => {}
        }
    }
}
