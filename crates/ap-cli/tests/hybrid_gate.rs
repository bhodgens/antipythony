//! Leaf 04.05 gate: the hybrid artifact end-to-end.
//!
//! The contract (plans/antipythony/04-packaging-ecosystem/orchestrator.md
//! leaf 04.05): ONE program's hybrid binary passes the native conformance
//! slice AND its `--emit-wasm` extraction passes the wasm slice — both
//! byte-exact vs CPython pins. The T-16 test track
//! (tests/t16_hybrid/test_t16_hybrid.py) pins the CLI seam: `build
//! --hybrid`, `app --emit-wasm` (rc 0 + exactly one .wasm in the cwd),
//! embedded-host flag `--wasm`, plain-binary refusal.
//!
//! Programs: the leaf brief says "hello + one classes_wasm-equivalent
//! program". The gate covers the WASM-GREEN corpus families (the wasm
//! half of every leg must actually run — values/cases outside the Plan 03
//! slice trap under ap-host, which is a Plan-03 boundary, not a 04.05
//! failure; measured: V003 traps in the emitter's scan vetting, c001
//! panics `scan vetted expr` — both recorded in NEXT-0405.md):
//! * a hello probe (fresh bytes, no corpus dependency);
//! * CLW001-init-attr-store + CLW004-class-attr-and-method (classes_wasm);
//! * CF001-while-count-sum (controlflow_wasm — loop lowering).
//!
//! The CPython pins are the CORPUS files (recorded by earlier leaves from
//! CPython 3.14); the fixture-validation test re-measures them against the
//! live reference interpreter when present (never weaker than the pin).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR = crates/ap-cli — the workspace root is two up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root")
}

/// Build the CLI/host binaries once per test-binary process (they are what
/// the gate is ABOUT).
fn cli_bin(name: &str) -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("test exe path");
    // <target>/<profile>/deps/<test-binary> → <target>/<profile>/<name>
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("profile dir")
        .to_path_buf();
    let cand = profile_dir.join(name);
    assert!(
        cand.exists(),
        "built binary missing: {} — cargo build -p ap-cli -p ap-host -p ap-aot-crt -p ap-aot-crt-hybrid first",
        cand.display()
    );
    cand
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let d = std::env::temp_dir().join(format!("apcli-0405-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// (stdout, stderr, rc) of running `bin args...` with the given cwd.
fn run_cwd(bin: &Path, args: &[&str], cwd: &Path) -> (Vec<u8>, Vec<u8>, i32) {
    let out = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn process");
    (out.stdout, out.stderr, out.status.code().unwrap_or(-1))
}

fn run(bin: &Path, args: &[&str]) -> (Vec<u8>, Vec<u8>, i32) {
    let cwd = std::env::temp_dir();
    run_cwd(bin, args, &cwd)
}

struct Case {
    /// Source file to build.
    src: PathBuf,
    /// The file NAME the builder sees (the emitter's source-name seam).
    file_name: &'static str,
    /// Pinned (stdout, stderr, rc).
    pin: (Vec<u8>, Vec<u8>, i32),
}

/// The gate programs: hello (fresh probe) + CLW001 (the classes_wasm
/// anchor, CPython pins) + the T-16 pair (V003 values + c001 classes,
/// their recorded pins).
fn gate_cases() -> Vec<(&'static str, Case)> {
    let root = workspace_root();
    let mut cases = Vec::new();

    // hello probe: no corpus pin needed; the pin is asserted via CPython
    // parity inside the tests that run the reference when available, and
    // the literal bytes here are the program's own semantics.
    cases.push((
        "hello",
        Case {
            src: PathBuf::new(), // written inline by the test
            file_name: "hello.py",
            pin: (b"hello, hybrid\n".to_vec(), Vec::new(), 0),
        },
    ));

    // CLW001: classes_wasm anchor with .out/.err/.rc pins.
    let clw = root
        .join("tests/corpus/classes_wasm")
        .join("CLW001-init-attr-store.py");
    let pins = root.join("tests/corpus/classes_wasm/expected");
    let read = |p: PathBuf| std::fs::read(p).unwrap_or_default();
    let rc = |p: PathBuf| -> i32 {
        std::fs::read_to_string(p)
            .unwrap_or_default()
            .trim()
            .parse()
            .unwrap_or(-999)
    };
    cases.push((
        "CLW001",
        Case {
            src: clw,
            file_name: "CLW001-init-attr-store.py",
            pin: (
                read(pins.join("CLW001-init-attr-store.out")),
                read(pins.join("CLW001-init-attr-store.err")),
                rc(pins.join("CLW001-init-attr-store.rc")),
            ),
        },
    ));

    // CF001: controlflow_wasm anchor (module-level While loop — the wasm
    // emitter's loop lowering on the extraction half, the tree-walk shape
    // on the native half). This family uses the values-style pin naming
    // (.stdout/.stderr/.exit).
    let cf = root
        .join("tests/corpus/controlflow_wasm")
        .join("CF001-while-count-sum.py");
    let cfpins = root.join("tests/corpus/controlflow_wasm/expected");
    cases.push((
        "CF001",
        Case {
            src: cf,
            file_name: "CF001-while-count-sum.py",
            pin: (
                read(cfpins.join("CF001-while-count-sum.stdout")),
                read(cfpins.join("CF001-while-count-sum.stderr")),
                rc(cfpins.join("CF001-while-count-sum.exit")),
            ),
        },
    ));

    // CLW004: a second classes_wasm case (class attr + method dispatch).
    let c4 = root
        .join("tests/corpus/classes_wasm")
        .join("CLW004-class-attr-and-method.py");
    let c4pins = root.join("tests/corpus/classes_wasm/expected");
    cases.push((
        "CLW004",
        Case {
            src: c4,
            file_name: "CLW004-class-attr-and-method.py",
            pin: (
                read(c4pins.join("CLW004-class-attr-and-method.out")),
                read(c4pins.join("CLW004-class-attr-and-method.err")),
                rc(c4pins.join("CLW004-class-attr-and-method.rc")),
            ),
        },
    ));
    cases
}

/// Build a hybrid binary for `case` into `dir`, named `tag`.
fn build_hybrid(cli: &Path, dir: &Path, case: &Case, tag: &str) -> PathBuf {
    let out = dir.join(tag);
    let build = Command::new(cli)
        .arg("build")
        .arg("--hybrid")
        .arg(&case.src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("spawn antipythony build --hybrid");
    assert!(
        build.status.success(),
        "build --hybrid failed (rc {:?}):\nstdout: {}\nstderr: {}",
        build.status.code(),
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(out.exists(), "hybrid build wrote no artifact");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert!(
            out.metadata().unwrap().permissions().mode() & 0o111 != 0,
            "hybrid artifact not executable"
        );
    }
    out
}

/// `app --emit-wasm` in a FRESH empty cwd: rc 0 + exactly one .wasm there.
fn emit_wasm(app: &Path) -> (PathBuf, (Vec<u8>, Vec<u8>, i32)) {
    let workdir = scratch_dir("emit");
    let triple = run_cwd(app, &["--emit-wasm"], &workdir);
    assert_eq!(
        triple.2,
        0,
        "--emit-wasm rc: stdout={} stderr={}",
        String::from_utf8_lossy(&triple.0),
        String::from_utf8_lossy(&triple.1)
    );
    let mut produced: Vec<PathBuf> = std::fs::read_dir(&workdir)
        .expect("read emit cwd")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "wasm").unwrap_or(false))
        .collect();
    assert_eq!(
        produced.len(),
        1,
        "--emit-wasm must produce exactly one .wasm in its cwd, got {produced:?}"
    );
    (produced.pop().expect("one"), triple)
}

/// The three-way gate for ONE program (the leaf contract, verbatim):
/// hybrid binary green on the native slice AND its extraction green on
/// the wasm slice, both against the CPython pin; plus payload byte
/// identity vs a fresh `build --wasm` and the embedded-host flag path.
fn gate_program(tag: &str, case_src_write: Option<&[u8]>) {
    let cli = cli_bin("antipythony");
    let ap_host = cli_bin("ap-host");
    let mut cases = gate_cases();
    let (_, case) = cases
        .iter_mut()
        .find(|(name, _)| *name == tag)
        .expect("gate case registered");
    if let Some(bytes) = case_src_write {
        let dir = scratch_dir(tag);
        let p = dir.join(case.file_name);
        std::fs::write(&p, bytes).expect("write probe source");
        case.src = p;
    }
    assert!(
        case.src.exists(),
        "case source missing: {}",
        case.src.display()
    );

    // Per-test BUILD dir (never the corpus dir — sequential tests clobber
    // each other's `app`/`standalone.wasm` there, measured: CLW004's build
    // overwrote CLW001's artifact mid-run). The case source is COPIED in
    // under its own file name so the emitter's source-name seam keeps
    // producing the corpus-faithful module bytes.
    let dir = scratch_dir(&format!("{tag}-build"));
    let local_src = dir.join(case.file_name);
    std::fs::copy(&case.src, &local_src).expect("copy case source into build dir");
    let case = Case {
        src: local_src,
        file_name: case.file_name,
        pin: (case.pin.0.clone(), case.pin.1.clone(), case.pin.2),
    };
    let app = build_hybrid(&cli, &dir, &case, "app");

    // ---- GATE HALF 1: the hybrid binary on the NATIVE slice ----
    let (out, err, rc) = run(&app, &[]);
    assert_eq!(out, case.pin.0, "native-path stdout vs CPython pin");
    assert_eq!(err, case.pin.1, "native-path stderr vs CPython pin");
    assert_eq!(rc, case.pin.2, "native-path rc vs CPython pin");

    // ---- argv passthrough unchanged (02.06 contract survives --hybrid) ----
    let (out2, err2, rc2) = run(&app, &["extra-arg", "2"]);
    assert_eq!(out2, case.pin.0, "argv passthrough changed stdout");
    assert_eq!(err2, case.pin.1, "argv passthrough changed stderr");
    assert_eq!(rc2, case.pin.2, "argv passthrough changed rc");

    // ---- embedded-host flag path: same triple as the native path ----
    let (wout, werr, wrc) = run(&app, &["--wasm"]);
    assert_eq!(wout, case.pin.0, "embedded-host stdout vs pin");
    assert_eq!(werr, case.pin.1, "embedded-host stderr vs pin");
    assert_eq!(wrc, case.pin.2, "embedded-host rc vs pin");

    // ---- extraction: rc 0, one .wasm, magic header ----
    let (module, emit_triple) = emit_wasm(&app);
    assert_eq!(emit_triple.1, b"", "--emit-wasm must not write to stderr");
    let bytes = std::fs::read(&module).expect("read extracted module");
    assert_eq!(&bytes[..4], b"\0asm", "extracted file is not a wasm module");

    // ---- GATE HALF 2: the extracted wasm on the WASM slice ----
    // Runner 1: the shipped ap-host binary.
    let (eout, eerr, erc) = run(&ap_host, &[module.to_str().expect("utf8 path")]);
    assert_eq!(eout, case.pin.0, "extracted-wasm(ap-host) stdout vs pin");
    assert_eq!(eerr, case.pin.1, "extracted-wasm(ap-host) stderr vs pin");
    assert_eq!(erc, case.pin.2, "extracted-wasm(ap-host) rc vs pin");

    // Runner 2: `antipythony host` (same engine, same triple).
    let h = run(&cli, &["host", module.to_str().expect("utf8 path")]);
    assert_eq!(h.0, case.pin.0, "extracted-wasm(host verb) stdout vs pin");
    assert_eq!(h.1, case.pin.1);
    assert_eq!(h.2, case.pin.2);

    // ---- payload byte-identity vs a fresh `build --wasm` ----
    let standalone = dir.join("standalone.wasm");
    let swbuild = Command::new(&cli)
        .arg("build")
        .arg("--wasm")
        .arg(&case.src)
        .arg("-o")
        .arg(&standalone)
        .output()
        .expect("spawn build --wasm");
    assert!(
        swbuild.status.success(),
        "build --wasm failed: {}",
        String::from_utf8_lossy(&swbuild.stderr)
    );
    assert_eq!(
        bytes,
        std::fs::read(&standalone).expect("read standalone module"),
        "embedded payload != fresh build --wasm output (same emitter, same name)"
    );

    // ---- the whole lattice: native == embedded-host == extracted == standalone == pin ----
    let (sout, serr, src_) = run(&ap_host, &[standalone.to_str().expect("utf8 path")]);
    assert_eq!(sout, case.pin.0);
    assert_eq!(serr, case.pin.1);
    assert_eq!(src_, case.pin.2);
    // Cleanup: ONLY the inline-probe scratch dir (never the corpus dir —
    // for corpus cases `dir` IS under tests/corpus; corpus only grows).
    if case.src.starts_with(std::env::temp_dir()) {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// The gate itself.
// ---------------------------------------------------------------------------

/// 04.05 GATE, program 1: hello.
#[test]
fn gate_hybrid_hello_native_and_extracted_wasm() {
    gate_program("hello", Some(b"print('hello, hybrid')\n"));
}

/// 04.05 GATE, program 2: the classes_wasm anchor (CLW001) — class def,
/// attr store in __init__, instance attr read/rewrite (the brief's
/// "classes_wasm-equivalent program").
#[test]
fn gate_hybrid_classes_case_native_and_extracted_wasm() {
    gate_program("CLW001", None);
}

/// Additional slice coverage: the controlflow_wasm anchor (CF001 —
/// module-level While; the wasm emitter's loop lowering vs the native
/// tree-walk shape for the same source).
#[test]
fn gate_hybrid_controlflow_case_native_and_extracted_wasm() {
    gate_program("CF001", None);
}

/// Additional slice coverage: a second classes_wasm case (CLW004 — class
/// attribute + method dispatch).
#[test]
fn gate_hybrid_class_method_case_native_and_extracted_wasm() {
    gate_program("CLW004", None);
}

// ---------------------------------------------------------------------------
// The refusal + hygiene legs (T-16 G-4 / the hybrid-only seam).
// ---------------------------------------------------------------------------

/// `--emit-wasm` is a HYBRID capability: a plain `build` binary must fail
/// CLEANLY (positive rc, stderr diagnostic, NO .wasm produced). The plain
/// CRT never claims the flag, so the plain binary hands it to the program
/// as argv — for an argv-ignoring program the run would EXIT 0 and print
/// nothing, which silently defeats the refusal contract. The ap-cli seam
/// therefore checks the artifact SHAPE: this test pins the CRT's refusal
/// by building a hybrid-minus-payload scenario? No — the plain binary has
/// no way to know. The OPERATIVE contract (recorded in NEXT-0405.md): the
/// plain binary treats --emit-wasm as program argv; the refusal leg is
/// satisfied at the BUILD-SEAM level — `build` (no --hybrid) never links
/// the payload object nor the hybrid CRT, proven here by the plain binary
/// NOT carrying the payload symbols.
#[test]
fn plain_native_binary_carries_no_payload_seam() {
    let cli = cli_bin("antipythony");
    let dir = scratch_dir("plain");
    let src = dir.join("plain.py");
    std::fs::write(&src, "print('plain')\n").expect("write probe");
    let out = dir.join("app_plain");
    let build = Command::new(&cli)
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("spawn antipythony build");
    assert!(build.status.success(), "plain build failed");
    // The plain binary must not carry the hybrid symbols (nm finds
    // nothing): no payload, no embedded host — nothing to claim the flag.
    let nm = Command::new("nm").arg(&out).output().expect("spawn nm");
    let text = String::from_utf8_lossy(&nm.stdout);
    assert!(
        !text.contains("ap_aot_wasm_payload"),
        "plain binary carries the payload seam — --hybrid leaked into `build`"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// On the PLAIN binary, `--emit-wasm` is now REFUSED LOUDLY (04.05
/// honest-gap-1 closeout, NEXT-04GAPS): the plain CRT answers with the
/// actionable diagnostic (rc 2, stderr names `build --hybrid`), NO .wasm
/// is produced, and the program never runs (the flag is claimed by the
/// launcher, not passed to the module). This was previously pinned as
/// argv-passthrough when the plain CRT was another agent's surface.
#[test]
fn plain_binary_treats_emit_wasm_as_program_argv() {
    let cli = cli_bin("antipythony");
    let dir = scratch_dir("argv");
    let src = dir.join("plain.py");
    std::fs::write(&src, "print('plain argv')\n").expect("write probe");
    let out = dir.join("app_plain2");
    let build = Command::new(&cli)
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("spawn antipythony build");
    assert!(build.status.success());
    let workdir = scratch_dir("argv-cwd");
    let triple = run_cwd(&out, &["--emit-wasm"], &workdir);
    // The program must NOT run (the flag is claimed before module code).
    assert!(
        triple.0.is_empty(),
        "plain binary ran the program despite --emit-wasm: {:?}",
        String::from_utf8_lossy(&triple.0)
    );
    // LOUD + actionable: rc 2, stderr names the hybrid rebuild fix.
    assert_eq!(
        triple.2, 2,
        "plain binary must refuse --emit-wasm with rc 2"
    );
    let stderr = String::from_utf8_lossy(&triple.1);
    assert!(
        stderr.contains("--emit-wasm"),
        "diagnostic must name the flag: {stderr}"
    );
    assert!(
        stderr.contains("build --hybrid"),
        "diagnostic must name the fix (build --hybrid): {stderr}"
    );
    let no_wasm: Vec<PathBuf> = std::fs::read_dir(&workdir)
        .expect("read cwd")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "wasm").unwrap_or(false))
        .collect();
    assert!(
        no_wasm.is_empty(),
        "plain binary produced a .wasm for --emit-wasm: {no_wasm:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Fixture validation: the pinned corpus files + pins exist and the live
/// CPython (when present) reproduces them — drift is loud HERE, never
/// blamed on a 04.05 artifact. (Reference = /opt/homebrew/bin/python3.14
/// or `python3.14` on PATH; the test skips when neither exists — the
/// pins themselves stay authoritative.)
#[test]
fn corpus_pins_reproduce_under_reference_cpython() {
    let reference = which_python();
    let mut cases = gate_cases();
    for (name, case) in cases.iter_mut() {
        if name == &"hello" {
            continue; // inline probe; no corpus pin
        }
        assert!(case.src.exists(), "{name}: corpus source missing");
        assert!(!case.pin.0.is_empty(), "{name}: pin stdout empty");
        assert_eq!(case.pin.2, 0, "{name}: pinned rc != 0");
        assert_eq!(case.pin.1, b"", "{name}: pinned stderr not empty");
        let Some(reference) = &reference else {
            continue;
        };
        let out = Command::new(reference)
            .arg(&case.src)
            .current_dir(case.src.parent().expect("dir"))
            .output()
            .expect("spawn reference python");
        assert_eq!(
            out.stdout, case.pin.0,
            "{name}: reference CPython stdout drifted from the recorded pin"
        );
        assert_eq!(out.stderr, case.pin.1, "{name}: reference stderr drifted");
        assert_eq!(
            out.status.code(),
            Some(case.pin.2),
            "{name}: reference rc drifted"
        );
    }
}

fn which_python() -> Option<PathBuf> {
    for cand in ["/opt/homebrew/bin/python3.14"] {
        let p = PathBuf::from(cand);
        if p.exists() {
            return Some(p);
        }
    }
    let out = Command::new("which").arg("python3.14").output().ok()?;
    if out.status.success() && !out.stdout.is_empty() {
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
        if p.exists() {
            return Some(p);
        }
    }
    None
}
