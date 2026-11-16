// Run PROBE env source and dump the observable triple (leaf 03.03 diag).
#[test]
fn diag_run033() {
    let src = std::env::var("PROBE").unwrap_or_default();
    let (m, _) = ap_parse::lower_source(&src).unwrap();
    let bytes = ap_codegen_wasm::lower_module(&m).unwrap();
    std::fs::write("/tmp/min.wasm", &bytes).unwrap();
    use wasmparser::{Validator, WasmFeatures};
    if let Err(e) = Validator::new_with_features(
        WasmFeatures::default() | WasmFeatures::GC | WasmFeatures::FUNCTION_REFERENCES | WasmFeatures::EXCEPTIONS,
    ).validate_all(&bytes) {
        println!("VALERR={} at {:?}", e, e.offset());
        return;
    }
    match ap_host::run(&bytes, 256 * 1024) {
        Ok(o) => println!("OUT={:?} ERR={:?} rc={}", o.stdout, o.stderr, o.exit_code),
        Err(e) => println!("RUNERR-DEBUG={:?}", e),
    }
}
