use ap_codegen_wasm::lower_module;

#[test]
fn diag_corpus_slice() {
    let dir = "/Users/caimlas/git/antipythony/tests/corpus/values";
    let cases = ["V001-bigint-exactness", "V047-truthiness-table", "V055-hash-eq-invariant",
                 "V008-bool-is-int", "V060-repr-int-decimal", "V056-hash-int-contract",
                 "V050-not-bool", "V051-eq-vs-identity", "V052-cross-type-numeric-eq",
                 "V054-chained-comparisons", "V059-hashable-types", "V057-hash-float-contract",
                 "V058-salted-hash-masked", "V005-true-division", "V003-floordiv-mod-signs",
                 "V002-power-rules", "V004-divmod-mixed", "V007-int-float-promotion",
                 "V010-round-bankers", "V011-int-string-parsing", "V012-str-codepoints-errors"];
    let only: Option<String> = std::env::var("DIAG_ONLY").ok();
    for c in cases {
        if let Some(o) = &only {
            if !c.contains(o.as_str()) { continue; }
        }
        let path = format!("{}/{}.py", dir, c);
        let src = match std::fs::read_to_string(&path) { Ok(s) => s, Err(_) => { eprintln!("{} MISSING", c); continue; } };
        let (module, _scope) = match ap_parse::lower_source(&src) {
            Ok(m) => m,
            Err(e) => { eprintln!("{} LOWER-SRC-ERR {:?}", c, e); continue; }
        };
        let bytes = match lower_module(&module) {
            Ok(b) => b,
            Err(ap_codegen_wasm::CodegenError::Unsupported(w)) => { eprintln!("{} UNSUPPORTED {}", c, w); continue; }
            Err(e) => { eprintln!("{} LOWER-ERR {:?}", c, e); continue; }
        };
        let _ = std::fs::write("/tmp/v001.wasm", &bytes);
        let _ = std::fs::write("/tmp/dbg3.wasm", &bytes);
        match ap_host::run(&bytes, 64 * 1024) {
            Ok(o) => eprintln!("{} OUT={:?} ERR={:?} rc={}", c, o.stdout, o.stderr, o.exit_code),
            Err(e) => eprintln!("{} RUN-ERR {} {}", c, {
                use wasmtime::wasmparser::Validator;
                let feats = wasmtime::wasmparser::WasmFeatures::default()
                    | wasmtime::wasmparser::WasmFeatures::GC
                    | wasmtime::wasmparser::WasmFeatures::FUNCTION_REFERENCES
                    | wasmtime::wasmparser::WasmFeatures::EXCEPTIONS;
                match Validator::new_with_features(feats).validate_all(&bytes) {
                    Err(ve) => format!("(validate: {})", ve),
                    Ok(_) => String::new(),
                }
            }, e),
        }
    }
}
