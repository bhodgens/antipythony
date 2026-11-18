// minimal while-yield probe
#[test]
fn minwh() {
    let src = "def g(n):\n    i = 0\n    while i < n:\n        yield i\n        i = i + 1\nit = g(3)\nprint(next(it))\nprint(next(it))\n";
    let (module, _) = ap_parse::lower_source(src).unwrap();
    let bytes = ap_codegen_wasm::lower_module_ex(&module, Some("minwh")).unwrap();
    std::fs::write("/tmp/minwh.wasm", &bytes).unwrap();
    {
        use wasmparser::Validator;
        let feats = wasmparser::WasmFeatures::default()
            | wasmparser::WasmFeatures::GC
            | wasmparser::WasmFeatures::FUNCTION_REFERENCES
            | wasmparser::WasmFeatures::EXCEPTIONS;
        if let Err(e) = Validator::new_with_features(feats).validate_all(&bytes) {
            println!("VALIDATE-ERR {e}");
            return;
        }
    }
    match ap_host::run(&bytes, 256 * 1024) {
        Ok(out) => println!("stdout {:?} exit {}", out.stdout, out.exit_code),
        Err(e) => println!("HOST-ERR {e:?}"),
    }
}
