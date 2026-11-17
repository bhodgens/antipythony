// GE001-GE005 + MA001 emit+validate+host-run probe (03.04c main loop):
// each corpus case: parse -> lower -> validate -> ap-host run -> print the
// observed triple vs the pins. The iteration driver until gates go green.
#[test]
fn ge_ma_probe() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    let cases: &[(&str, &str)] = &[
        ("generators_wasm", "GE001-yield-next-exhaust"),
        ("generators_wasm", "GE002-send-values"),
        ("generators_wasm", "GE003-throw-close"),
        ("generators_wasm", "GE004-yield-from"),
        ("generators_wasm", "GE005-comprehensions-genexp"),
        ("match_wasm", "MA001-patterns-slice"),
    ];
    let dump = std::env::var("PROBE_DUMP").is_ok();
    for (fam, stem) in cases {
        let src =
            std::fs::read_to_string(root.join("tests/corpus").join(fam).join(format!("{stem}.py")))
                .unwrap();
        let (module, _) = match ap_parse::lower_source(&src) {
            Ok(m) => m,
            Err(e) => {
                println!("{stem}: PARSE-ERR {e:?}");
                continue;
            }
        };
        let bytes = match ap_codegen_wasm::lower_module_ex(&module, Some(stem)) {
            Ok(b) => b,
            Err(ap_codegen_wasm::CodegenError::Unsupported(w)) => {
                println!("{stem}: UNSUPPORTED {w}");
                continue;
            }
            Err(e) => {
                println!("{stem}: LOWER-ERR {e:?}");
                continue;
            }
        };
        if dump {
            let name = stem.split('-').next().unwrap();
            std::fs::write(format!("/tmp/{name}.wasm"), &bytes).unwrap();
        }
        // validate
        {
            use wasmparser::Validator;
            let feats = wasmparser::WasmFeatures::default()
                | wasmparser::WasmFeatures::GC
                | wasmparser::WasmFeatures::FUNCTION_REFERENCES
                | wasmparser::WasmFeatures::EXCEPTIONS;
            if let Err(e) = Validator::new_with_features(feats).validate_all(&bytes) {
                println!("{stem}: VALIDATE-ERR {e}");
                continue;
            }
        }
        match ap_host::run(&bytes, 256 * 1024) {
            Ok(out) => {
                let exp = root.join("tests/corpus").join(fam).join("expected");
                let want_out =
                    std::fs::read(exp.join(format!("{stem}.stdout"))).unwrap();
                let want_err = std::fs::read(exp.join(format!("{stem}.stderr"))).unwrap();
                let want_exit: i32 = std::fs::read_to_string(exp.join(format!("{stem}.exit")))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                let ok = out.stdout.as_bytes() == want_out.as_slice()
                    && out.stderr.as_bytes() == want_err.as_slice()
                    && out.exit_code == want_exit;
                if ok {
                    println!("{stem}: PASS");
                } else {
                    println!(
                        "{stem}: MISMATCH\n  want stdout {:?}\n  got  stdout {:?}\n  want stderr {:?}\n  got  stderr {:?}\n  want exit {} got exit {}",
                        String::from_utf8_lossy(&want_out),
                        out.stdout,
                        String::from_utf8_lossy(&want_err),
                        out.stderr,
                        want_exit,
                        out.exit_code
                    );
                }
            }
            Err(e) => println!("{stem}: HOST-ERR {e}"),
        }
    }
}
