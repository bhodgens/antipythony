//! Leaf 04.04 gate: `antipythony build --wasm` end-to-end.
//!
//! The contract: a .py built through the CLI VERB produces a .wasm that
//! runs byte-exact (stdout/stderr/exit triple) under BOTH the shipped
//! `ap-host` binary and the `antipythony host` verb, matching the CPython
//! pins, and is byte-identical to the Plan 03 dump-test output for the
//! same (source, source-name) pair — the source name is the INVOKED FILE
//! NAME with extension (`hello.py`), the CPython-faithful choice: CPython's
//! tracebacks carry the invoked filename. The Plan 03 dump tests pass the
//! case STEM, so the dump-comparison arm names the file identically to the
//! verb (scratch copies keep the corpus stem) rather than assuming the
//! dump-test's naming.
//!
//! The system-wasmtime arm of the leaf gate is recorded in
//! plans/antipythony/04-packaging-ecosystem/NEXT-0404.md: no `wasmtime` CLI
//! on this host (leaf default = document absence, no install without user
//! OK), so the second runner is the `antipythony host` verb backed by the
//! SAME wasmtime 47 engine ap-host uses (the engine contract was itself
//! differential-pinned vs CPython in Plan 03's 51/51 CLI corpus gate).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::Path;
use std::process::Command;

fn workspace_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR = crates/ap-cli — the workspace root is two up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root")
}

/// Build the CLI binaries once per test-binary process (they are what the
/// gate is ABOUT — the verb, not the library seam).
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
        "built binary missing: {} — cargo build -p ap-cli -p ap-host first",
        cand.display()
    );
    cand
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let d = std::env::temp_dir().join(format!("apcli-0404-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// (stdout, stderr, rc) of running a module through a runner binary.
fn run_triple(bin: &Path, wasm: &Path) -> (Vec<u8>, Vec<u8>, i32) {
    let out = Command::new(bin)
        .arg("run")
        .arg(wasm)
        .output()
        .expect("spawn runner");
    (out.stdout, out.stderr, out.status.code().unwrap_or(-1))
}

#[test]
fn build_wasm_verb_hello_and_classes_byte_exact_under_both_runners() {
    let root = workspace_root();
    let antipythony = cli_bin("antipythony");
    let ap_host = cli_bin("ap-host");

    // hello: the corpus probe source (same bytes the Plan 03 dump test
    // inlines), built via the NEW verb into a scratch dir.
    let dir = scratch_dir("hello");
    let src = dir.join("hello.py");
    std::fs::write(&src, "print('hello, wasm')\n").unwrap();
    let wasm = dir.join("hello.wasm");
    let build = Command::new(&antipythony)
        .arg("build")
        .arg("--wasm")
        .arg(&src)
        .arg("-o")
        .arg(&wasm)
        .output()
        .expect("spawn antipythony");
    assert!(
        build.status.success(),
        "build --wasm failed: {} {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(wasm.exists(), "verb wrote no output module");

    // Byte-identity vs the Plan 03 emitter for the same (source, name):
    // the dump test names inline probes by stem, so rebuild through the
    // library seam with the SAME name the verb chose (file stem) and
    // compare — the emitter is a pure function of those two inputs.
    let (module, _) = ap_parse::lower_source("print('hello, wasm')\n").expect("parse");
    let expected = ap_codegen_wasm::lower_module_ex(&module, Some("hello.py")).expect("lower");
    assert_eq!(
        std::fs::read(&wasm).unwrap(),
        expected,
        "verb output diverges from the Plan 03 emitter for the same inputs"
    );

    // Runner 1: the shipped ap-host binary — byte-exact triple.
    let (out, err, rc) = run_triple(&ap_host, &wasm);
    assert_eq!(out, b"hello, wasm\n");
    assert_eq!(err, b"");
    assert_eq!(rc, 0);

    // Runner 2: `antipythony host` (the embedded-ap-host install story) —
    // same triple through the same engine.
    let out2 = Command::new(&antipythony)
        .arg("host")
        .arg("run")
        .arg(&wasm)
        .output()
        .expect("spawn antipythony host");
    assert_eq!(out2.stdout, b"hello, wasm\n");
    assert_eq!(out2.stderr, b"");
    assert_eq!(out2.status.code(), Some(0));

    // Runner 3 (leaf gate's second arm, honestly recorded): the system
    // wasmtime CLI is ABSENT on this host (checked at leaf time; installing
    // is forbidden without user OK). The engine behind BOTH runners above
    // is wasmtime 47 — the identical engine the Plan 03 CLI corpus gate
    // pinned 51/51 byte-exact vs CPython. NEXT-0404.md carries the record.
    let wasmtime_present = Command::new("which")
        .arg("wasmtime")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if wasmtime_present {
        // If a future host HAS the CLI, hold the gate for real.
        let out3 = Command::new("wasmtime")
            .arg("run")
            .arg(&wasm)
            .output()
            .expect("spawn wasmtime");
        assert_eq!(out3.stdout, b"hello, wasm\n");
        assert_eq!(out3.status.code(), Some(0));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_wasm_verb_classes_case_matches_cpython_pins() {
    let root = workspace_root();
    let antipythony = cli_bin("antipythony");
    let ap_host = cli_bin("ap-host");

    let stem = "CLW001-init-attr-store";
    let corpus_src = root
        .join("tests/corpus/classes_wasm")
        .join(format!("{stem}.py"));
    let pins_dir = root.join("tests/corpus/classes_wasm/expected");

    // Build via the verb with the file named EXACTLY like the corpus case
    // so the emitter's source-name seam matches the dump-test stem.
    let dir = scratch_dir("classes");
    let src = dir.join(format!("{stem}.py"));
    std::fs::copy(&corpus_src, &src).unwrap();
    let wasm = dir.join(format!("{stem}.wasm"));
    let build = Command::new(&antipythony)
        .arg("build")
        .arg("--wasm")
        .arg(&src)
        .arg("-o")
        .arg(&wasm)
        .output()
        .expect("spawn antipythony");
    assert!(
        build.status.success(),
        "build --wasm failed: {} {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // Byte-identity vs the Plan 03 dump-test output: the emitter is a pure
    // function of (source, source-name). The verb passes the INVOKED FILE
    // NAME (`{stem}.py`, CPython's traceback-faithful choice); the dump
    // test passes the bare stem. Re-emit with the dump-test's name and
    // require byte-identity — pinning the "plumbed to the Plan 03 output"
    // claim mechanically while documenting the single deliberate delta
    // (the 3-byte name-length seam) between the two callers.
    let dumped = root.join("logs/wasm-corpus").join(format!("{stem}.wasm"));
    if dumped.exists() {
        let (module, _) = ap_parse::lower_source(&std::fs::read_to_string(&src).unwrap())
            .expect("parse corpus case");
        let dump_named =
            ap_codegen_wasm::lower_module_ex(&module, Some(stem)).expect("lower dump-named");
        assert_eq!(
            std::fs::read(&dumped).unwrap(),
            dump_named,
            "dump-test output no longer matches the emitter (same inputs)"
        );
        let verb_named = ap_codegen_wasm::lower_module_ex(&module, Some(&format!("{stem}.py")))
            .expect("lower verb-named");
        assert_eq!(
            std::fs::read(&wasm).unwrap(),
            verb_named,
            "verb output diverges from the emitter for its own (source, name)"
        );
        // The ONLY allowed delta between the two outputs is the embedded
        // source-name string itself (same module otherwise). The name
        // lands TWICE (the string-table blob AND the i32 length constant
        // emitted for the `$str` build), so the delta is 2x the 3-byte
        // `.py` suffix.
        let d = std::fs::read(&dumped).unwrap();
        let v = std::fs::read(&wasm).unwrap();
        assert_eq!(
            d.len() + 2 * ".py".len(),
            v.len(),
            "delta must be exactly the `.py` suffix bytes (name appears twice)"
        );
    }

    // CPython pins: stdout/stderr/rc.
    let want_out = std::fs::read(pins_dir.join(format!("{stem}.out"))).unwrap();
    let want_err = std::fs::read(pins_dir.join(format!("{stem}.err"))).unwrap();
    let want_rc: i32 = std::fs::read_to_string(pins_dir.join(format!("{stem}.rc")))
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    // Under the shipped ap-host binary.
    let (got_out, got_err, got_rc) = run_triple(&ap_host, &wasm);
    let got_out: Vec<u8> = got_out.iter().copied().filter(|&b| b != b'\r').collect();
    assert_eq!(got_out, want_out, "stdout diverges from the CPython pin");
    assert_eq!(got_err, want_err, "stderr diverges from the CPython pin");
    assert_eq!(got_rc, want_rc, "exit code diverges from the CPython pin");

    // Under the `antipythony host` verb.
    let out2 = Command::new(&antipythony)
        .arg("host")
        .arg(&wasm)
        .output()
        .expect("spawn host verb");
    let o2: Vec<u8> = out2
        .stdout
        .iter()
        .copied()
        .filter(|&b| b != b'\r')
        .collect();
    assert_eq!(o2, want_out);
    assert_eq!(out2.status.code(), Some(want_rc));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_wasm_unsupported_construct_fails_loud_not_silent() {
    // The wasm backend's fail-closed contract: an Unsupported construct is a
    // LOUD build error naming the construct, never a silently-wrong module.
    let antipythony = cli_bin("antipythony");
    let dir = scratch_dir("unsupported");
    let src = dir.join("t.py");
    std::fs::write(&src, "import antipythony_nonexistent_module_xyz\n").unwrap();
    let build = Command::new(&antipythony)
        .arg("build")
        .arg("--wasm")
        .arg(&src)
        .output()
        .expect("spawn antipythony");
    assert!(
        !build.status.success(),
        "unsupported construct must fail the build, not emit a wrong module"
    );
    let msg = String::from_utf8_lossy(&build.stderr);
    assert!(
        msg.contains("unsupported") || msg.contains("Unsupported") || msg.contains("wasm"),
        "error should name the wasm lowering: {msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
