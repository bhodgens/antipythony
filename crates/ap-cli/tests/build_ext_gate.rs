//! Leaf 10.03 gate: `antipythony build-ext` end-to-end (the graduated verb,
//! both example extensions, byte-exact vs CPython 3.14).
//!
//! Contract under test:
//! 1. the verb compiles a C extension source against include/apx_python.h
//!    into a dlopen-able .so (module-name inference from PyInit_<name>);
//! 2. `import <name>` under antipythony native routes through the ap-ce
//!    dlopen seam and the registries;
//! 3. stdout/stderr/rc are byte-identical with CPython 3.14 running the
//!    SAME source compiled against REAL Python.h (the 10.02 dual-compile
//!    differential-oracle pattern, now gated in CI shape);
//! 4. the module-name-mismatch fail-loud path;
//! 5. the spike extension (10.02) still round-trips byte-exact after the
//!    10.03 shim ABI fixes (ParseTuple 'i' now writes a C int).
//!
//! Reference discovery + fenced interpreter spawns: tests/common (the
//! repo-wide differential-suite discipline). Header discovery for the
//! CPython arm: `python3.14-config --includes/--ldflags` (skipped cleanly
//! when the tool is absent — same policy as the python3.14 skip).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn antipythony() -> String {
    bin_path("antipythony")
}

/// Resolve a workspace bin honoring CARGO_TARGET_DIR (shared-target builds).
fn bin_path(name: &str) -> String {
    let p = match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::Path::new(&d).join("debug").join(name),
        Err(_) => std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target-0304c/debug") // worktree target-dir convention
            .join(name),
    };
    p.to_string_lossy().into_owned()
}

/// Scratch dir for one test (inside std::env::temp_dir; unique per test).
fn scratch_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let d = std::env::temp_dir().join(format!("apcli-1003-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// The repo's include dir (the headers the verb builds against).
fn repo_include() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("include")
        .canonicalize()
        .expect("repo include/")
}

/// `python3.14-config` present? (needed for the CPython-arm compile)
fn py_config() -> Option<String> {
    for prog in ["python3.14-config"] {
        if Command::new(prog)
            .arg("--exists")
            .output()
            .is_ok_and(|o| o.status.success())
            || Command::new(prog)
                .arg("--includes")
                .output()
                .is_ok_and(|o| o.status.success())
        {
            return Some(prog.to_string());
        }
    }
    None
}

struct CpyCompile {
    includes: String,
    ldflags: String,
}

fn cpy_compile_args(pycfg: &str) -> Option<CpyCompile> {
    let inc = Command::new(pycfg).arg("--includes").output().ok()?;
    let ld = Command::new(pycfg)
        .arg("--ldflags")
        .arg("--embed")
        .output()
        .ok()?;
    if !(inc.status.success() && ld.status.success()) {
        return None;
    }
    Some(CpyCompile {
        includes: String::from_utf8_lossy(&inc.stdout).trim().to_string(),
        ldflags: String::from_utf8_lossy(&ld.stdout).trim().to_string(),
    })
}

/// Compile `src` BOTH ways into `dir` (antipythony lib<name>.so via the
/// verb; CPython <name>.so via cc + Python.h). Returns (ap_so, cpy_so).
fn build_both(dir: &Path, src_name: &str) -> (PathBuf, PathBuf, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root");
    let src = root.join("tests").join("ce-examples").join(src_name);
    assert!(src.is_file(), "missing example source: {}", src.display());

    // Antipythony arm: the VERB (this is what the gate is about). The
    // default output convention is lib<name>.so BESIDE THE SOURCE; the
    // gate relocates with -o (cross-dir install is exactly the case -o
    // exists for).
    let stem = src_name.strip_suffix(".c").expect(".c source");
    let ap_so = dir.join(format!("lib{stem}.so"));
    let out = Command::new(antipythony())
        .arg("build-ext")
        .arg(&src)
        .arg("-o")
        .arg(&ap_so)
        .output()
        .expect("spawn antipythony build-ext");
    assert!(
        out.status.success(),
        "build-ext failed: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(ap_so.is_file(), "verb wrote no {}", ap_so.display());

    // CPython arm: same source, REAL Python.h (differential oracle).
    let py = common::reference_or_skip().expect("reference resolved by caller");
    let pycfg = py_config().unwrap_or_else(|| format!("{py}-config"));
    let args = cpy_compile_args(&pycfg).expect("python3.14-config --includes/--ldflags failed");
    let cpy_so = dir.join(format!("{stem}.so"));
    let cc_out = Command::new("cc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("-DAPX_CPYTHON_BUILD")
        .args(args.includes.split_whitespace())
        .arg("-O2")
        .arg(&src)
        .arg("-o")
        .arg(&cpy_so)
        .args(args.ldflags.split_whitespace())
        .output()
        .expect("spawn cc (CPython arm)");
    assert!(
        cc_out.status.success(),
        "CPython-arm cc failed: {}",
        String::from_utf8_lossy(&cc_out.stderr)
    );
    (ap_so, cpy_so, py.to_string())
}

/// Run `probe` under both interpreters with the respective .so importable
/// (antipythony via cwd, CPython via sys.path[0] == cwd) and require the
/// byte-identical stdout/stderr/rc triple.
fn assert_differential(py: &str, dir: &Path, probe_name: &str, ap_so: &Path, _cpy_so: &Path) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root");
    let probe = root.join("tests").join("ce-examples").join(probe_name);

    // Antipythony: fenced (repo-wide interpreter discipline); cwd = the
    // scratch dir so the dlopen probe finds lib<name>.so.
    let ap_out = common::fenced_output(
        Command::new(antipythony())
            .arg("run")
            .arg(&probe)
            .current_dir(dir)
            .env("LANG", "C")
            .env_remove("APX_EXT_DIR"),
    );
    // CPython: plain spawn (reference interpreter; not the gate's subject).
    let cpy = Command::new(py)
        .arg(&probe)
        .current_dir(dir)
        .env("LANG", "C")
        .env("PYTHONPATH", dir)
        .output()
        .expect("spawn python3.14");

    let ap_rc = ap_out.status.code().unwrap_or(-1);
    let cpy_rc = cpy.status.code().unwrap_or(-1);
    assert_eq!(
        ap_out.stdout,
        cpy.stdout,
        "{probe_name} stdout diverges (antipythony vs CPython 3.14)\n--- antipythony ---\n{}",
        String::from_utf8_lossy(&ap_out.stdout)
    );
    assert_eq!(
        ap_out.stderr,
        cpy.stderr,
        "{probe_name} stderr diverges\n--- antipythony ---\n{}\n--- cpython ---\n{}",
        String::from_utf8_lossy(&ap_out.stderr),
        String::from_utf8_lossy(&cpy.stderr)
    );
    assert_eq!(ap_rc, cpy_rc, "{probe_name} exit code diverges");
    assert!(
        !ap_out.stdout.is_empty(),
        "probe produced no output ({} built at {})",
        probe_name,
        ap_so.display()
    );
}

#[test]
fn build_ext_geom_example_byte_exact_vs_cpython() {
    let dir = scratch_dir("geom");
    let (ap_so, cpy_so, py) = build_both(&dir, "geom.c");
    assert_differential(&py, &dir, "geom_probe.py", &ap_so, &cpy_so);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_ext_spike_still_byte_exact_after_1003_shim_fixes() {
    let dir = scratch_dir("spike");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root");
    let spike_src = root.join("tests").join("ce-spike").join("spike.c");
    let spike_so = dir.join("libspike.so");
    let out = Command::new(antipythony())
        .arg("build-ext")
        .arg(&spike_src)
        .arg("-o")
        .arg(&spike_so)
        .output()
        .expect("spawn antipythony build-ext");
    assert!(out.status.success(), "spike build-ext failed");

    // CPython arm for the SAME source (the spike's dual-compile shape; the
    // compat header lives beside it). Without this, `import spike` fails
    // under CPython and the differential asserts nothing.
    let py = common::reference_or_skip();
    let Some(py) = py else {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let pycfg = py_config().unwrap_or_else(|| format!("{py}-config"));
    let args = cpy_compile_args(&pycfg).expect("python3.14-config --includes/--ldflags failed");
    let cpy_so = dir.join("spike.so");
    let cc_out = Command::new("cc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("-DAPX_CPYTHON_BUILD")
        .args(args.includes.split_whitespace())
        .arg("-O2")
        .arg(&spike_src)
        .arg("-o")
        .arg(&cpy_so)
        .args(args.ldflags.split_whitespace())
        .output()
        .expect("spawn cc (CPython arm)");
    assert!(
        cc_out.status.success(),
        "CPython-arm cc failed: {}",
        String::from_utf8_lossy(&cc_out.stderr)
    );
    let probe_src = r#"import sys
sys.path.insert(0, ".")
import spike
print(spike.add_two(2, 40))
print(spike.greet("antipythony"))
try:
    spike.fails("boom")
except ValueError as e:
    print("caught:", e)
p = spike.Point(3, 4)
print(type(p).__name__)
print(p.scaled(10))
"#;
    let probe = dir.join("spike_gate_probe.py");
    std::fs::write(&probe, probe_src).unwrap();

    let ap_out = common::fenced_output(
        Command::new(antipythony())
            .arg("run")
            .arg(&probe)
            .current_dir(&dir)
            .env("LANG", "C"),
    );
    let cpy = Command::new(py)
        .arg(&probe)
        .current_dir(&dir)
        .env("LANG", "C")
        .env("PYTHONPATH", &dir)
        .output()
        .expect("spawn python3.14");
    assert_eq!(ap_out.stdout, cpy.stdout, "spike stdout diverges");
    assert_eq!(ap_out.stderr, cpy.stderr, "spike stderr diverges");
    assert_eq!(
        ap_out.status.code(),
        cpy.status.code(),
        "spike exit code diverges"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_ext_module_name_mismatch_fails_loud() {
    // PyInit_othername in wrongname.c: the verb must refuse (the loader
    // probes lib<stem>.so for PyInit_<stem>; a mismatch is a silent
    // never-importable .so if built anyway).
    let antipythony = antipythony();
    let dir = scratch_dir("mismatch");
    let src = dir.join("wrongname.c");
    std::fs::write(
        &src,
        r#"#include "apx_python.h"
PyObject *PyInit_othername(void) { return 0; }
"#,
    )
    .unwrap();
    let out = Command::new(&antipythony)
        .arg("build-ext")
        .arg(&src)
        .current_dir(&dir)
        .env("APX_INCLUDE", repo_include())
        .output()
        .expect("spawn antipythony");
    assert!(
        !out.status.success(),
        "PyInit/file mismatch must fail the build, not emit a dead .so"
    );
    let msg = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        msg.contains("PyInit_othername") && msg.contains("wrongname"),
        "error should name both the symbol and the file: {msg}"
    );
    assert!(
        !dir.join("libwrongname.so").exists(),
        "no artifact must be written for a rejected build"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_ext_emit_header_installs_apx_python_h() {
    let antipythony = antipythony();
    let dir = scratch_dir("header");
    let src = dir.join("hdrmod.c");
    std::fs::write(
        &src,
        r#"#include "apx_python.h"
PyObject *PyInit_hdrmod(void) { return 0; }
"#,
    )
    .unwrap();
    let out = Command::new(&antipythony)
        .arg("build-ext")
        .arg(&src)
        .arg("--emit-header")
        .current_dir(&dir)
        .env("APX_INCLUDE", repo_include())
        .output()
        .expect("spawn antipythony");
    assert!(
        out.status.success(),
        "build-ext --emit-header failed: {out:?}"
    );
    let hdr = dir.join("apx_python.h");
    assert!(hdr.is_file(), "header not installed beside the output");
    let built = std::fs::read_to_string(&hdr).expect("read installed header");
    let orig = std::fs::read_to_string(repo_include().join("apx_python.h")).expect("read orig");
    assert_eq!(
        built, orig,
        "installed header diverges from the repo header"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
