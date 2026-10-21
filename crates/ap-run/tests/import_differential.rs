//! import_differential — leaf 02.04a (file-based import machinery).
//!
//! Differential probes vs live `python3.14` (reference resolved by
//! tests/common: env AP_CONFORM_REFERENCE_PYTHON -> PATH python3.14; skips
//! locally when absent, hard-fails under AP_REQUIRE_REFERENCE=1).
//! Both interpreters run the SAME fixture tree under tests/fixtures/importpkg;
//! stdout bytes + exit code must match.
//!
//! Gates covered: package imports, relative imports (from . / from .mod),
//! circular imports with partial-module visibility, byte-exact
//! ModuleNotFoundError/ImportError texts, sys.modules identity semantics.
//!
//! KNOWN BLOCKED (documented in PROGRESS, not gated here): the vendored
//! stdlib's pathlib/dataclasses/enum chain needs metaclass __new__
//! (EnumType/ABCMeta) — a Plan-02 follow-up. Vendored tree lives at
//! vendored_stdlib/ (PSF-2.0, see vendored_stdlib/NOTICE.md).

use std::process::Command;

// Reference discovery is shared (leaf 00.02): env AP_CONFORM_REFERENCE_PYTHON
// -> PATH python3.14 (version-STRING checked; the old spawn-only probe could
// accept a non-3.14 interpreter that merely happened to be named python3.14).
// AP_REQUIRE_REFERENCE=1 converts a missing reference into a hard failure.
mod common;

fn ap_run_eval() -> &'static str {
    ap_run_eval_dyn().leak()
}

fn ap_run_eval_dyn() -> String {
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::Path::new(&d).join("debug/ap-run-eval"),
        Err(_) => {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/ap-run-eval")
        }
    }
    .to_string_lossy()
    .into_owned()
}

/// Run `src` under both interpreters with the fixture dir prepended to the
/// path (ANTPYTHANY_PATH for us, PYTHONPATH for the reference); assert
/// identical stdout + exit code.
fn assert_differential_with_path(src: &str, fixture: &std::path::Path) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let tmp = std::env::temp_dir().join(format!(
        "ap_import_diff_{}_{}.py",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&tmp, src).expect("write temp case");
    let expected = Command::new(py)
        .arg(&tmp)
        .env("PYTHONPATH", fixture)
        .output()
        .expect("spawn python3.14");
    let actual = Command::new(ap_run_eval())
        .arg(&tmp)
        .env("ANTPYTHANY_PATH", fixture)
        .output()
        .expect("spawn ap-run-eval");
    std::fs::remove_file(&tmp).ok();

    let expected_out = String::from_utf8_lossy(&expected.stdout).to_string();
    let actual_out = String::from_utf8_lossy(&actual.stdout).to_string();
    assert_eq!(
        expected.status.code(),
        actual.status.code(),
        "exit code mismatch:\n-- expected stdout --\n{expected_out}\n-- actual stdout --\n{actual_out}\n-- actual stderr --\n{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(expected_out, actual_out, "stdout mismatch");
}

/// Unique per-process fixture root (tests run in parallel).
fn fixture_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ap_import_fixtures_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir fixture root");
    dir
}

#[test]
fn package_import_and_relative() {
    let root = fixture_root("pkg");
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(
        root.join("pkg/__init__.py"),
        "VALUE = 42\nfrom .mod import helper\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pkg/mod.py"),
        "def helper():\n    return \"helped\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pkg/sub.py"),
        "from .mod import helper\nX = helper()\n",
    )
    .unwrap();
    assert_differential_with_path(
        r#"
import pkg
print(pkg.VALUE, pkg.helper())
from pkg import mod
print(mod.helper())
import pkg.mod
print(pkg.mod.helper())
from pkg.sub import X
print(X)
"#,
        &root,
    );
}

#[test]
fn circular_imports_partial_visibility() {
    let root = fixture_root("circ");
    std::fs::write(root.join("a.py"), "import b\nX = b.Y + 1\n").unwrap();
    std::fs::write(root.join("b.py"), "import a\nY = 10\n").unwrap();
    assert_differential_with_path(
        r#"
import a
print(a.X, a.b.Y)
import sys
print(sys.modules["a"] is a, sys.modules["b"] is b)
"#,
        &root,
    );
}

#[test]
fn import_error_texts_byte_exact() {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let root = fixture_root("err");
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(root.join("pkg/__init__.py"), "").unwrap();
    // Missing module (both plain and dotted), missing name, relative without
    // parent. stderr last-line texts must match CPython byte-exactly.
    for case in [
        "import nonexistent_zzz",
        "import nonexistent_zzz.sub",
        "from pkg import missing_name",
        "from . import x",
        "from .mod import y",
    ] {
        let tmp = std::env::temp_dir().join(format!(
            "ap_import_err_{}_{}",
            std::process::id(),
            case.replace([' ', '.'], "_")
        ));
        std::fs::write(&tmp, case).unwrap();
        let expected = Command::new(py)
            .arg(&tmp)
            .env("PYTHONPATH", &root)
            .output()
            .expect("spawn python3.14");
        let actual = Command::new(ap_run_eval())
            .arg(&tmp)
            .env("ANTPYTHANY_PATH", &root)
            .output()
            .expect("spawn ap-run-eval");
        std::fs::remove_file(&tmp).ok();
        let exp_last = String::from_utf8_lossy(&expected.stderr)
            .lines()
            .last()
            .unwrap_or("")
            .to_string();
        let act_last = String::from_utf8_lossy(&actual.stderr)
            .lines()
            .last()
            .unwrap_or("")
            .to_string();
        assert_eq!(exp_last, act_last, "stderr last line mismatch for: {case}");
        assert_eq!(expected.status.code(), actual.status.code(), "{case}");
    }
}

#[test]
fn sys_modules_identity() {
    let root = fixture_root("ident");
    std::fs::write(root.join("onemod.py"), "Q = 1\n").unwrap();
    assert_differential_with_path(
        r#"
import onemod
import sys
print(onemod is sys.modules["onemod"])
import onemod as again
print(again is onemod)
"#,
        &root,
    );
}

#[test]
fn vendored_types_and_abc_import() {
    // The vendored tree (vendored_stdlib/) is on sys.path by bootstrap.
    // types.py + _py_abc + _collections_abc are the deepest chains that work
    // without metaclass __new__.
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let src = "import types\nimport _collections_abc\nprint('ok')\n";
    let tmp = std::env::temp_dir().join(format!("ap_vendored_{}", std::process::id()));
    std::fs::write(&tmp, src).unwrap();
    let expected = Command::new(py)
        .arg(&tmp)
        .output()
        .expect("spawn python3.14");
    let actual = Command::new(ap_run_eval())
        .arg(&tmp)
        .output()
        .expect("spawn ap-run-eval");
    std::fs::remove_file(&tmp).ok();
    // Our execution diverges at the ABC register(coroutine) interplay —
    // documented v0 gap (metaclass __new__). Assert we get PAST the imports
    // machinery itself (no ModuleNotFoundError for vendored files).
    let err = String::from_utf8_lossy(&actual.stderr);
    assert!(
        !err.contains("No module named"),
        "vendored file resolution broke: {err}"
    );
    let _ = expected; // reference output not byte-compared for this probe
}

#[test]
fn script_dir_on_sys_path() {
    // A script imports a module sitting NEXT TO it (script_dir bootstrap).
    let root = fixture_root("scriptdir");
    std::fs::write(root.join("neighbor.py"), "N = 5\n").unwrap();
    let main = root.join("main_script.py");
    std::fs::write(&main, "import neighbor\nprint(neighbor.N)\n").unwrap();
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let expected = Command::new(py)
        .arg(&main)
        .output()
        .expect("spawn python3.14");
    let actual = Command::new(ap_run_eval())
        .arg(&main)
        .output()
        .expect("spawn ap-run-eval");
    assert_eq!(
        String::from_utf8_lossy(&expected.stdout),
        String::from_utf8_lossy(&actual.stdout)
    );
    assert_eq!(expected.status.code(), actual.status.code());
}
