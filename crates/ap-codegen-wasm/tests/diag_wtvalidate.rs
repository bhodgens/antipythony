// Wasmtime-validation triage: name the function + print wasmtime's full error
// for /tmp/min.wasm (wasmparser accepts; wasmtime rejects — find the fn).
#[test]
fn diag_wtvalidate() {
    let bytes = std::fs::read("/tmp/min.wasm").unwrap();
    let engine = wasmtime::Engine::new(
        wasmtime::Config::new().wasm_exceptions(true).wasm_gc(true),
    )
    .unwrap();
    match wasmtime::Module::new(&engine, &bytes) {
        Ok(_) => eprintln!("WTVAL: module compiles fine"),
        Err(e) => eprintln!("WTVAL-ERR: {e}"),
    }
}
