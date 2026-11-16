//! Per-case probe: lowers each controlflow_wasm case, reporting panics.
#[test]
fn cf_probe_each() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap();
    let fam = root.join("tests/corpus/controlflow_wasm");
    for p in std::fs::read_dir(&fam).unwrap() {
        let p = p.unwrap().path();
        if p.extension().map(|e| e != "py").unwrap_or(true) { continue; }
        let stem = p.file_stem().unwrap().to_str().unwrap().to_string();
        let src = std::fs::read_to_string(&p).unwrap();
        let (m, _) = match ap_parse::lower_source(&src) {
            Ok(m) => m,
            Err(e) => { eprintln!("{stem}: PARSE-ERR {e:?}"); continue; }
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ap_codegen_wasm::lower_module(&m))) {
            Ok(Ok(bytes)) => {
                match ap_host::run(&bytes, 64 * 1024) {
                    Ok(o) => eprintln!("{stem}: OUT={:?} rc={}", o.stdout, o.exit_code),
                    Err(e) => eprintln!("{stem}: RUN-ERR {}", e),
                }
            }
            Ok(Err(ap_codegen_wasm::CodegenError::Unsupported(w))) => eprintln!("{stem}: UNSUPPORTED {w}"),
            Ok(Err(e)) => eprintln!("{stem}: LOWER-ERR {e:?}"),
            Err(_) => eprintln!("{stem}: LOWER-PANIC"),
        }
    }
}
