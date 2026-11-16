//! Leaf 03.03c differential gate: every `tests/corpus/classes_wasm` and
//! `tests/corpus/exceptions_wasm` case goes parse → lower_module → ap-host
//! run, and the observable triple (stdout, stderr, exit) must match the
//! CPython 3.14 pins byte-exactly (controlflow_wasm_gate pattern).
//!
//! Pins live in each family's `expected/` (.out/.err/.rc), recorded by the
//! reference interpreter per tests/README.md. The corpus only grows — a
//! mismatch is a REAL regression, never a pin edit.

use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|p| p.to_path_buf())
        .expect("crate at <root>/crates/ap-codegen-wasm")
}

fn run_family(root: &Path, fam: &str, failures: &mut Vec<String>) -> usize {
    let dir = root.join("tests/corpus").join(fam);
    let exp = dir.join("expected");
    let mut passed = 0usize;
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("family dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
        .collect();
    paths.sort();
    for p in paths {
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&p).unwrap();
        let want_out = std::fs::read(exp.join(format!("{stem}.out"))).expect("pin .out");
        let want_err = std::fs::read(exp.join(format!("{stem}.err"))).expect("pin .err");
        let want_rc: i32 = std::fs::read_to_string(exp.join(format!("{stem}.rc")))
            .expect("pin .rc")
            .trim()
            .parse()
            .expect("pin rc int");
        let (module, _) = match ap_parse::lower_source(&src) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{fam}/{stem}: parse error {e:?}"));
                continue;
            }
        };
        let bytes = match ap_codegen_wasm::lower_module_ex(&module, Some(&stem)) {
            Ok(b) => b,
            Err(ap_codegen_wasm::CodegenError::Unsupported(w)) => {
                failures.push(format!("{fam}/{stem}: Unsupported({w})"));
                continue;
            }
            Err(e) => {
                failures.push(format!("{fam}/{stem}: lower error {e:?}"));
                continue;
            }
        };
        match ap_host::run(&bytes, 256 * 1024) {
            Ok(out) => {
                let ok = out.stdout.as_bytes() == want_out.as_slice()
                    && out.stderr.as_bytes() == want_err.as_slice()
                    && out.exit_code == want_rc;
                if ok {
                    passed += 1;
                } else {
                    failures.push(format!(
                        "{fam}/{stem}: MISMATCH\n  want out {:?}\n  got  out {:?}\n  want err {:?}\n  got  err {:?}\n  want rc {}\n  got  rc {}",
                        String::from_utf8_lossy(&want_out),
                        out.stdout,
                        String::from_utf8_lossy(&want_err),
                        out.stderr,
                        want_rc,
                        out.exit_code,
                    ));
                }
            }
            Err(e) => failures.push(format!("{fam}/{stem}: host error {e}")),
        }
    }
    passed
}

#[test]
fn classes_exceptions_wasm_differential() {
    let root = repo_root();
    let mut failures = Vec::new();
    let passed = run_family(&root, "classes_wasm", &mut failures)
        + run_family(&root, "exceptions_wasm", &mut failures);
    eprintln!(
        "classes/exceptions wasm differential: {} passed, {} failed",
        passed,
        failures.len(),
    );
    assert!(
        failures.is_empty(),
        "byte-exact failures vs CPython 3.14 pins:\n{}",
        failures.join("\n")
    );
}
