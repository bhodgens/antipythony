//! Leaf 03.02c differential gate: every `tests/corpus/controlflow_wasm`
//! case goes parse → lower_module → ap-host run, and the observable triple
//! (stdout, stderr, exit) must match the CPython 3.14 pins byte-exactly.
//!
//! Pins live in `tests/corpus/controlflow_wasm/expected/` (.stdout /
//! .stderr / .exit), recorded by the reference interpreter per the corpus
//! rules (tests/README.md). This gate is the T-12 control-flow slice: the
//! corpus only grows — a mismatch is a REAL regression, never a pin edit.

use std::path::Path;

/// (case stem, allow-xfail reason) — every case added to the corpus MUST
/// appear here; an empty reason means the case is expected to pass.
const CASES: &[(&str, &str)] = &[
    ("CF001-while-count-sum", ""),
    ("CF002-while-continue-odds", ""),
    ("CF003-while-break-skips-else", ""),
    ("CF004-while-else-runs-on-normal-exit", ""),
    ("CF005-while-mod-sign-table", ""),
    ("CF006-if-elif-else-first-true", ""),
    ("CF007-if-truthiness-table", ""),
    ("CF008-and-or-values", ""),
    ("CF009-ifexp-lazy", ""),
    ("CF010-call-static-arity", ""),
    ("CF011-call-defaults", ""),
    ("CF012-call-dynamic-name", ""),
    ("CF013-recursion", ""),
    ("CF014-closure-counter", ""),
    ("CF015-closure-late-binding-loop", ""),
    ("CF016-closure-factory-independent", ""),
    ("CF017-lambda-values", ""),
    ("CF018-nonlocal-shared-cell", ""),
];

fn repo_root() -> std::path::PathBuf {
    // tests/ lives at the repo root; the gate runs from the crate dir via
    // CARGO_MANIFEST_DIR (worktree-safe: the corpus is checked out with the
    // branch, so the worktree copy is authoritative for this leaf).
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|p| p.to_path_buf())
        .expect("crate at <root>/crates/ap-codegen-wasm")
}

#[test]
fn controlflow_wasm_differential() {
    let root = repo_root();
    let fam = root.join("tests/corpus/controlflow_wasm");
    let exp = fam.join("expected");
    assert!(fam.is_dir(), "corpus family missing: {}", fam.display());

    let mut passed = 0usize;
    let mut xfailed = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (stem, xfail_reason) in CASES {
        let src = std::fs::read_to_string(fam.join(format!("{stem}.py")))
            .unwrap_or_else(|e| panic!("{stem}: corpus case missing: {e}"));
        let want_stdout = std::fs::read(exp.join(format!("{stem}.stdout")))
            .unwrap_or_else(|e| panic!("{stem}: pin missing: {e}"));
        let want_stderr = std::fs::read(exp.join(format!("{stem}.stderr")))
            .unwrap_or_else(|e| panic!("{stem}: pin missing: {e}"));
        let want_exit: i32 = std::fs::read_to_string(exp.join(format!("{stem}.exit")))
            .unwrap_or_else(|e| panic!("{stem}: pin missing: {e}"))
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("{stem}: bad exit pin: {e}"));

        let (module, _) = match ap_parse::lower_source(&src) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{stem}: parse error {e:?}"));
                continue;
            }
        };
        let bytes = match ap_codegen_wasm::lower_module(&module) {
            Ok(b) => b,
            Err(ap_codegen_wasm::CodegenError::Unsupported(w)) => {
                if xfail_reason.is_empty() {
                    failures.push(format!("{stem}: lowered as Unsupported({w})"));
                } else {
                    xfailed += 1;
                }
                continue;
            }
            Err(e) => {
                failures.push(format!("{stem}: lower error {e:?}"));
                continue;
            }
        };
        match ap_host::run(&bytes, 256 * 1024) {
            Ok(out) => {
                let ok = out.stdout.as_bytes() == want_stdout
                    && out.stderr.as_bytes() == want_stderr
                    && out.exit_code == want_exit;
                if ok {
                    passed += 1;
                } else if !xfail_reason.is_empty() {
                    xfailed += 1;
                } else {
                    failures.push(format!(
                        "{stem}: MISMATCH\n  want stdout {:?}\n  got  stdout {:?}\n  want stderr {:?}\n  got  stderr {:?}\n  want exit {}\n  got  exit {}",
                        String::from_utf8_lossy(&want_stdout),
                        out.stdout,
                        String::from_utf8_lossy(&want_stderr),
                        out.stderr,
                        want_exit,
                        out.exit_code,
                    ));
                }
            }
            Err(e) => {
                if !xfail_reason.is_empty() {
                    xfailed += 1;
                } else {
                    failures.push(format!("{stem}: host error {e}"));
                }
            }
        }
    }

    eprintln!(
        "controlflow_wasm differential: {} passed, {} xfailed, {} failed (of {})",
        passed,
        xfailed,
        failures.len(),
        CASES.len(),
    );
    assert!(
        failures.is_empty(),
        "byte-exact failures vs CPython 3.14 pins:\n{}",
        failures.join("\n")
    );
}
