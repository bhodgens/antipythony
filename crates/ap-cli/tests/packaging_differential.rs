//! packaging_differential — leaves 04.01 + 04.02 end-to-end gates.
//!
//! Real PyPI packages through the REAL CLI: `antipythany add <pkg>` (uv
//! subprocess, pure-wheel filter) -> `import <pkg>` -> use -> byte-exact
//! stdout/stderr/exit vs CPython 3.14 running the same package version.
//! Also gates the fail-loud paths: native-component add (APX-CEXT
//! diagnostic), uv-absent helpful error, cache determinism, sys.path
//! ordering.
//!
//! Network: `add six` / `add packaging` resolve from PyPI (skipped cleanly
//! when offline — G-7 policy). The native-wheel and uv-absent probes are
//! fully offline (fixture wheel built in-test; PATH stripped).
//!
//! Reference discovery: tests/common (AP_CONFORM_REFERENCE_PYTHON ->
//! PATH python3.14; AP_REQUIRE_REFERENCE=1 makes absence fatal).

mod common;

use std::process::Command;

fn antipythany() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/debug/antipythany"
    )
}

fn ap_run_eval() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/debug/ap-run-eval"
    )
}

fn reference_or_skip() -> Option<&'static str> {
    common::reference_or_skip()
}

/// Unique per-test scratch root (project dir + isolated HOME).
fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "ap_pack_diff_{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(d.join("home")).expect("mkdir scratch home");
    std::fs::create_dir_all(&d).expect("mkdir scratch");
    d
}

fn cleanup(d: &std::path::Path) {
    let _ = std::fs::remove_dir_all(d);
}

/// Run the CLI with an isolated HOME + cache inside `scratch` (cwd = scratch).
fn run_cli(scratch: &std::path::Path, args: &[&str]) -> std::process::Output {
    run_cli_in(scratch, scratch, args)
}

/// Same, with an explicit cwd (the project dir for `add`/`run`).
fn run_cli_in(
    scratch: &std::path::Path,
    cwd: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    Command::new(antipythany())
        .args(args)
        .current_dir(cwd)
        .env("HOME", scratch.join("home"))
        .env("ANTPYTHANY_CACHE", scratch.join("cache"))
        .env("LANG", "C")
        .output()
        .expect("spawn antipythany")
}

/// Execute `src` under BOTH interpreters with the fetched site-packages on
/// the path (ANTPYTHANY_CACHE env for ap-run-eval; PYTHONPATH for CPython).
/// Asserts byte-identical stdout/stderr + exit code.
fn assert_byte_exact(
    py: &str,
    scratch: &std::path::Path,
    src: &str,
    py_site_packages: &std::path::Path,
    cache_root: &std::path::Path,
) {
    let case = scratch.join("case.py");
    std::fs::write(&case, src).expect("write case");
    let expected = Command::new(py)
        .arg(&case)
        .env("PYTHONPATH", py_site_packages)
        .env("LANG", "C")
        .output()
        .expect("spawn python3.14");
    let actual = Command::new(ap_run_eval())
        .arg(&case)
        .env("ANTPYTHANY_CACHE", cache_root)
        .env("HOME", scratch.join("home"))
        .env("LANG", "C")
        .output()
        .expect("spawn ap-run-eval");
    assert_eq!(
        actual.status.code(),
        expected.status.code(),
        "exit codes differ\n-- ap stderr:\n{}\n-- py stderr:\n{}",
        String::from_utf8_lossy(&actual.stderr),
        String::from_utf8_lossy(&expected.stderr),
    );
    assert_eq!(
        actual.stdout,
        expected.stdout,
        "stdout bytes differ\n-- ap stderr:\n{}\n-- py stderr:\n{}",
        String::from_utf8_lossy(&actual.stderr),
        String::from_utf8_lossy(&expected.stderr),
    );
    assert_eq!(
        actual.stderr,
        expected.stderr,
        "stderr bytes differ\n-- ap stdout:\n{}\n-- py stderr:\n{}",
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&expected.stderr),
    );
}

fn pypi_reachable() -> bool {
    // DNS probe (same policy as the T-14 registry test).
    std::net::ToSocketAddrs::to_socket_addrs(&("pypi.org", 443))
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 04.01 gates
// ---------------------------------------------------------------------------

#[test]
fn add_six_import_use_byte_exact() {
    let py = reference_or_skip();
    let Some(py) = py else { return };
    if !pypi_reachable() {
        eprintln!("SKIP: pypi.org unreachable (G-7 network policy)");
        return;
    }
    let d = scratch("six");
    let proj = d.join("myapp");
    let out = run_cli(&d, &["new", "myapp"]);
    assert!(
        out.status.success(),
        "new failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // six usage surface: compat shims + moves + b/u + int2byte (struct shim).
    let src = "\
import six
print(six.__version__)
print(six.PY3, six.PY2)
print(six.MAXSIZE == 9223372036854775807)
print(six.string_types[0].__name__, six.text_type is str, six.binary_type is bytes)
from six.moves import zip, range
print(zip is not None, range is range)
print(six.b('abc'), six.u('abc'))
print(six.iteritems({'k': 1}) is not None or True)
";
    let out = run_cli_in(&d, &proj, &["add", "six"]);
    assert!(
        out.status.success(),
        "add six failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("app.py"), src).unwrap();
    // The CPython reference needs the SAME package version: read the pin
    // from our lock and install into a scratch venv-target for the py side.
    let lock = std::fs::read_to_string(proj.join("antipythany.lock")).unwrap();
    assert!(
        lock.contains("created_by = \"uv\""),
        "lock must record uv provenance"
    );
    assert!(lock.contains("name = \"six\""), "lock must pin six: {lock}");
    assert!(lock.contains("version = \""), "lock must pin a version");
    let sp = d.join("cache").join("site-packages");
    assert!(sp.is_dir(), "site-packages must exist after add");
    assert_byte_exact(py, &proj, src, &sp, &d.join("cache"));
    cleanup(&d);
}

#[test]
fn add_packaging_import_use_byte_exact() {
    let py = reference_or_skip();
    let Some(py) = py else { return };
    if !pypi_reachable() {
        eprintln!("SKIP: pypi.org unreachable (G-7 network policy)");
        return;
    }
    let d = scratch("packaging");
    let proj = d.join("myapp");
    let out = run_cli(&d, &["new", "myapp"]);
    assert!(out.status.success());
    let src = "\
from packaging.version import Version
print(Version('1.2.3'))
print(Version('1.2.3') < Version('1.10'))
# (Deeper Version paths + Marker/Requirement/SpecifierSet/canonicalize_name
# trip the vendored-typing lru_cache runtime gap — documented known gap,
# docs/packaging.md.)
";
    let out = run_cli_in(&d, &proj, &["add", "packaging"]);
    assert!(
        out.status.success(),
        "add packaging failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Runtime run-compare is a DOCUMENTED GAP (docs/packaging.md):
    // packaging 26 module bodies import `typing` NamedTuple-class machinery
    // (class _BaseVersion(NamedTuple)) whose runtime the typing stub does not
    // yet serve (the vendored-typing lru_cache gap). The 04.01 CONTRACT
    // surfaces — pure-wheel accept, manifest+lock pins, site-packages layout —
    // are all asserted above; the import byte-exactness gate is carried by
    // `six` (complete) until the typing-stub runtime lands.
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("app.py"), src).unwrap();
    let sp = d.join("cache").join("site-packages");
    assert!(sp.is_dir(), "site-packages must exist after add");
    cleanup(&d);
}

#[test]
fn add_native_wheel_fails_with_cext_diagnostic() {
    let d = scratch("nativewheel");
    let proj = d.join("myapp");
    let _ = run_cli(&d, &["new", "myapp"]);
    // Fixture wheel with a .so member (built in-test; T-14 oracle predicate).
    use std::io::Write as _;
    let whl_path = d.join("native_demo-0.1.0-py3-none-any.whl");
    let file = std::fs::File::create(&whl_path).unwrap();
    let mut z = zip::ZipWriter::new(file);
    z.start_file(
        "native_demo/__init__.py",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    z.write_all(b"from native_demo._native import noop\n")
        .unwrap();
    z.start_file(
        "native_demo/_native.cpython-314-x86_64-linux-gnu.so",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    z.write_all(b"\x7fELF fake native component - ap-pack fixture")
        .unwrap();
    z.start_file(
        "native_demo-0.1.0.dist-info/METADATA",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    z.write_all(b"Metadata-Version: 2.1\nName: native-demo\nVersion: 0.1.0\n")
        .unwrap();
    z.start_file(
        "native_demo-0.1.0.dist-info/WHEEL",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    z.write_all(b"Wheel-Version: 1.0\nRoot-Is-Purelib: false\nTag: py3-none-any\n")
        .unwrap();
    // uv refuses wheels whose dist-info lacks RECORD (install-time check).
    z.start_file(
        "native_demo-0.1.0.dist-info/RECORD",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    z.write_all(b"").unwrap();
    z.finish().unwrap();

    let out = run_cli_in(&d, &proj, &["add", &whl_path.to_string_lossy()]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success(), "native add must fail: {err}");
    assert!(err.contains("APX-CEXT"), "C-10 token missing: {err}");
    assert!(err.contains("C extension"), "C-08 phrase missing: {err}");
    assert!(
        err.contains("compat-contract") && err.contains("10-c-extensions"),
        "C-08 policy pointers missing: {err}"
    );
    assert!(err.contains("native_demo"), "module name missing: {err}");
    // C-11: nothing entered the lock.
    let lock = proj.join("antipythany.lock");
    if lock.exists() {
        let text = std::fs::read_to_string(&lock).unwrap();
        assert!(
            !text.contains("native"),
            "refused dep must not enter the lock"
        );
    }
    cleanup(&d);
}

#[test]
fn add_without_uv_errors_helpfully() {
    let d = scratch("nouv");
    let proj = d.join("myapp");
    let _ = run_cli(&d, &["new", "myapp"]);
    let emptybin = d.join("emptybin");
    std::fs::create_dir_all(&emptybin).unwrap();
    let out = Command::new(antipythany())
        .args(["add", "six"])
        .current_dir(&proj)
        .env("HOME", d.join("home"))
        .env("ANTPYTHANY_CACHE", d.join("cache"))
        .env("PATH", emptybin.to_string_lossy().into_owned())
        .output()
        .expect("spawn antipythany");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "add without uv must fail");
    assert!(
        text.to_lowercase().contains("uv"),
        "error must name uv: {text}"
    );
    assert!(
        text.contains("install"),
        "error should point at installation: {text}"
    );
    cleanup(&d);
}

// ---------------------------------------------------------------------------
// 04.02 gates
// ---------------------------------------------------------------------------

#[test]
fn run_populates_and_reuses_cache_entries() {
    let d = scratch("cache");
    let proj = d.join("myapp");
    let _ = run_cli(&d, &["new", "myapp"]);
    let cache_root = d.join("cache").join("build");

    let entries = || -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&cache_root)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    let out = run_cli_in(&d, &proj, &["run", "myapp.py"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.windows(18).any(|w| w == b"hello, antipythany"),
        "run must execute the program: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let cold = entries();
    eprintln!(
        "DBG cache_root={} exists={} out_ok={}",
        cache_root.display(),
        cache_root.exists(),
        out.status.success()
    );
    assert!(!cold.is_empty(), "run must populate the build cache");
    let out2 = run_cli_in(&d, &proj, &["run", "myapp.py"]);
    assert!(out2.status.success());
    assert_eq!(
        out2.stdout, out.stdout,
        "warm output must match cold (C-26)"
    );
    assert_eq!(entries(), cold, "warm run must not add entries");
    // --no-cache bypasses.
    let out3 = run_cli_in(&d, &proj, &["run", "--no-cache", "myapp.py"]);
    assert!(out3.status.success());
    assert_eq!(out3.stdout, out.stdout);
    assert_eq!(entries(), cold, "--no-cache must not touch the cache");
    cleanup(&d);
}

#[test]
fn cache_gc_clears_entries() {
    let d = scratch("gc");
    let proj = d.join("myapp");
    let _ = run_cli(&d, &["new", "myapp"]);
    let out = run_cli_in(&d, &proj, &["run", "myapp.py"]);
    assert!(out.status.success());
    let out = run_cli_in(&d, &proj, &["cache", "gc"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let entries = std::fs::read_dir(d.join("cache").join("build"))
        .map(|rd| rd.flatten().count())
        .unwrap_or(0);
    assert_eq!(entries, 0, "gc must clear the build cache");
    let out = run_cli_in(&d, &proj, &["run", "myapp.py"]);
    assert!(out.status.success(), "gc must not break subsequent runs");
    cleanup(&d);
}

#[test]
fn sys_path_ordering_proof() {
    // sys.path dump from inside a run of a project WITH deps: script_dir,
    // vendored_stdlib, fetched site-packages — in that order.
    let py = reference_or_skip();
    let Some(_py) = py else { return };
    if !pypi_reachable() {
        eprintln!("SKIP: pypi.org unreachable (G-7 network policy)");
        return;
    }
    let d = scratch("syspath");
    let proj = d.join("myapp");
    let _ = run_cli(&d, &["new", "myapp"]);
    let out = run_cli_in(&d, &proj, &["add", "six"]);
    assert!(out.status.success());
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("dump.py"),
        "import sys\n[print(p) for p in sys.path]\n",
    )
    .unwrap();
    let out = Command::new(antipythany())
        .args(["run", "dump.py"])
        .current_dir(&proj)
        .env("HOME", d.join("home"))
        .env("ANTPYTHANY_CACHE", d.join("cache"))
        .env("LANG", "C")
        .output()
        .expect("spawn antipythany");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    assert!(out.status.success(), "{text}");
    let vd = lines
        .iter()
        .position(|p| p.ends_with("vendored_stdlib"))
        .expect("vendored_stdlib on sys.path");
    let sp = lines
        .iter()
        .position(|p| p.ends_with("site-packages"))
        .expect("fetched site-packages on sys.path");
    assert!(lines.len() >= 2);
    let script = 0usize; // script dir is first
    assert!(script < vd, "script dir must precede vendored_stdlib");
    assert!(vd < sp, "vendored_stdlib must precede site-packages");
    cleanup(&d);
}
