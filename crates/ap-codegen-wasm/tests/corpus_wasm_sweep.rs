
// Sweep: run classes/exceptions corpus cases through the wasm pipeline; report per-case status.
use std::path::Path;
fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors().nth(2).map(|p| p.to_path_buf()).expect("crate at <root>/crates/ap-codegen-wasm")
}
#[test]
fn corpus_wasm_sweep() {
    let root = repo_root();
    for fam in ["exceptions", "classes"] {
        let dir = root.join("tests/corpus").join(fam);
        for entry in std::fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().and_then(|e| e.to_str()) != Some("py") { continue; }
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&p).unwrap();
            let exp = dir.join("expected");
            let want_out = std::fs::read(exp.join(format!("{stem}.out")));
            let want_err = std::fs::read(exp.join(format!("{stem}.err")));
            let want_rc: i32 = std::fs::read_to_string(exp.join(format!("{stem}.rc")))
                .map(|s| s.trim().parse().unwrap_or(-999)).unwrap_or(-998);
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if let Ok(f) = std::env::var("CASE") { if f != name { continue; } }
            let (m, _) = match ap_parse::lower_source(&src) {
                Ok(m) => m,
                Err(e) => { eprintln!("{fam}/{name}: PARSE-ERR {e:?}"); continue; }
            };
            let src_name = format!("{name}");
            let bytes = match ap_codegen_wasm::lower_module_ex(&m, Some(&src_name)) {
                Ok(b) => b,
                Err(ap_codegen_wasm::CodegenError::Unsupported(w)) => {
                    eprintln!("{fam}/{name}: UNSUPPORTED {w}"); continue;
                }
                Err(e) => { eprintln!("{fam}/{name}: LOWER-ERR {e:?}"); continue; }
            };
            match ap_host::run(&bytes, 256 * 1024) {
                Ok(o) => {
                    let ok_out = want_out.as_ref().map(|w| o.stdout.as_bytes() == w.as_slice());
                    let ok_err = want_err.as_ref().map(|w| o.stderr.as_bytes() == w.as_slice());
                    let ok_rc = o.exit_code == want_rc;
                    if ok_out.unwrap_or(false) && ok_err.unwrap_or(false) && ok_rc {
                        eprintln!("{fam}/{name}: PASS");
                    } else {
                        eprintln!("{fam}/{name}: MISMATCH out={} err={} rc={} (want rc {want_rc})",
                            ok_out.unwrap_or(false), ok_err.unwrap_or(false), o.exit_code);
                    }
                }
                Err(e) => eprintln!("{fam}/{name}: HOST-ERR {}", format!("{}", e).lines().next().unwrap_or("")),
            }
        }
    }
}
