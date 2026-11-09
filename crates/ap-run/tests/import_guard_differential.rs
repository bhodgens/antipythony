//! import_guard_differential — leaf 02.02 (robustness-ci, review finding #16).
//!
//! Defensive path-segment guard in the file-import finder
//! (`find_on_path` / `find_submodule`, crates/ap-run/src/importer.rs):
//! module-name segments that are '', '.', '..', or contain '/' or '\'
//! must never be joined onto a sys.path directory. The replacement errors
//! are byte-pinned against live CPython 3.14 `__import__(name)` (level=0)
//! — the review's paraphrase "ImportError: invalid module name" is WRONG;
//! CPython 3.14 prints `ValueError: Empty module name` for the empty name
//! and `ModuleNotFoundError: No module named '...'` variants otherwise.
//!
//! Reference probe transcript (live, this host, python3.14 3.14.7 -S):
//!   __import__('')        -> ValueError: Empty module name            (exit 1)
//!   __import__('.')       -> ModuleNotFoundError: No module named '.'
//!   __import__('..')      -> ModuleNotFoundError: No module named '.'
//!   __import__('...')     -> ModuleNotFoundError: No module named '.'
//!   __import__('..a')     -> ModuleNotFoundError: No module named '.'
//!   __import__('../evil') -> ModuleNotFoundError: No module named '.'
//!   __import__('.a')      -> ModuleNotFoundError: No module named '.a'
//!   __import__('.a.b')    -> ModuleNotFoundError: No module named '.a'
//!   __import__('.os')     -> ValueError: Empty module name
//!   __import__('.json')   -> ValueError: Empty module name
//!   __import__('.os/')    -> ModuleNotFoundError: No module named '.os/'
//!   __import__('a..b')    -> ModuleNotFoundError: No module named 'a'
//!   __import__('a.b.')    -> ModuleNotFoundError: No module named 'a'
//!   __import__('os/..')   -> ModuleNotFoundError: No module named 'os/'
//!   __import__('/etc/..') -> ModuleNotFoundError: No module named '/etc/'
//!   __import__('a/b/..')  -> ModuleNotFoundError: No module named 'a/b/'
//!   __import__('a b')     -> ModuleNotFoundError: No module named 'a b'
//!   name = 'a'+chr(92)+'b' -> ModuleNotFoundError: No module named 'a\\b' (repr)
//!   name = "a'q"          -> ModuleNotFoundError: No module named "a'q"  (repr)
//!
//! Pre-guard antipythony behavior (fixed here): '__import__("")==OK' with
//! __path__ = [sys.path base dir] (empty segment joined the base dir and
//! matched it as a namespace package), '__import__("/etc/..")' imported
//! /etc/ as a namespace, '__import__("../../evil")' walked OUT of sys.path
//! entries into parent directories. Valid dotted imports must keep working.
//!
//! Deliberate scope cut: corpus cases (tests/corpus/stdlib/) are NOT added
//! per the leaf dispatch — corpus lints + traceability need spec-cite
//! review by a different agent; this Rust differential probe is the fence.
//! The "is not a package" suffix family (os.nope) is a documented
//! native-module fidelity divergence — see the leaf PROGRESS note.

use std::process::Command;

mod common;

fn ap_run_eval() -> String {
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::Path::new(&d).join("debug/ap-run-eval"),
        Err(_) => {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/ap-run-eval")
        }
    }
    .to_string_lossy()
    .into_owned()
}

fn write_case(tag: &str, src: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "ap_import_guard_{}_{}_{}.py",
        std::process::id(),
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("import_guard_differential.rs:65")
            .as_nanos()
    ));
    std::fs::write(&p, src).expect("write probe");
    p
}

/// Run one probe source under python3.14 (-S, same file bytes) and
/// ap-run-eval with the same extra sys.path fixture; assert stdout bytes,
/// stderr LAST LINE (traceback shapes differ by design — the exception
/// line is the pinned contract, per import_differential.rs precedent), and
/// exit code.
fn check_guard_case(tag: &str, src: &str, fixture: Option<&std::path::Path>) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let tmp = write_case(tag, src);
    let mut ref_cmd = Command::new(py);
    ref_cmd.arg("-S").arg(&tmp);
    // Interpreter side runs under the OOM fence (2026-11-06 mitigation 2).
    let mut ap_cmd = Command::new(ap_run_eval());
    ap_cmd.arg(&tmp);
    if let Some(f) = fixture {
        ref_cmd.env("PYTHONPATH", f);
        ap_cmd.env("ANTIPTYHONY_PATH", f);
    }
    let expected = ref_cmd.output().expect("spawn python3.14");
    let actual = common::fenced_output(&mut ap_cmd);
    std::fs::remove_file(&tmp).ok();

    let exp_out = String::from_utf8_lossy(&expected.stdout).to_string();
    let act_out = String::from_utf8_lossy(&actual.stdout).to_string();
    assert_eq!(
        expected.status.code(),
        actual.status.code(),
        "exit code mismatch for {tag}:\n-- ref stdout --\n{exp_out}\n-- ap stdout --\n{act_out}\n-- ap stderr --\n{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(exp_out, act_out, "stdout mismatch for {tag}");
    let exp_err = String::from_utf8_lossy(&expected.stderr)
        .lines()
        .last()
        .unwrap_or("")
        .to_string();
    let act_err = String::from_utf8_lossy(&actual.stderr)
        .lines()
        .last()
        .unwrap_or("")
        .to_string();
    assert_eq!(
        exp_err, act_err,
        "stderr last line mismatch for {tag}\n(ref source was uncaught?)"
    );
}

/// Python source that __import__s each shape (built with chr() for the
/// backslash/quote shapes so the FILE BYTES are identical under both
/// interpreters) and prints `type: message` or `OK`.
fn catch_probe(shapes: &[String]) -> String {
    let mut src = String::from(
        "import sys\n\
         def probe(name):\n\
         \x20   try:\n\
         \x20       __import__(name)\n\
         \x20       print('OK')\n\
         \x20   except BaseException as e:\n\
         \x20       print(type(e).__name__ + ': ' + str(e))\n",
    );
    for sh in shapes {
        src.push_str(&format!("probe({sh})\n"));
    }
    src
}

fn q(s: &str) -> String {
    // Python literal for the shape, valid in both parsers.
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

// ---------------------------------------------------------------- guard ---

#[test]
fn empty_name_is_value_error() {
    common::interpreter_gate();
    // CPython _sanity_check fires BEFORE any path lookup; pre-guard ap
    // "succeeded" and returned the sys.path base dir as a namespace pkg.
    check_guard_case("empty", &catch_probe(&[q("")]), None);
}

#[test]
fn dot_and_dotdot_prefix_family() {
    common::interpreter_gate();
    check_guard_case(
        "dots",
        &catch_probe(&[
            q("."),
            q(".."),
            q("..."),
            q("..a"),
            q("../evil"),
            q("..os.path"),
        ]),
        None,
    );
}

#[test]
fn leading_dot_single_name() {
    common::interpreter_gate();
    // '.x': unresolvable head -> ModuleNotFoundError with leading dot;
    // resolvable head ('.os', '.json', '.keyword') -> CPython's
    // Empty-module-name ValueError (empty partition head). Each shape runs
    // in a FRESH process: CPython's leading-dot quirk is sensitive to
    // sys.modules state from earlier probes in the same process.
    for sh in [
        q(".a"),
        q(".a.b"),
        q(".nonexistentpkg"),
        q(".nonexistentpkg.foo"),
        q(".os"),
        q(".json"),
        q(".keyword"),
        q(".os.path"),
        q(".os/"),
        q(".os."),
        q(".1"),
        q(".sys"),
        q(".builtins"),
    ] {
        check_guard_case("lead", &catch_probe(std::slice::from_ref(&sh)), None);
    }
}

#[test]
fn empty_and_separating_interior_segments() {
    common::interpreter_gate();
    // Dotted names with empty interior/trailing segments — CPython reports
    // the failing parent prefix chain.
    check_guard_case(
        "interior",
        &catch_probe(&[q("a..b"), q("a.b."), q("a.b..c"), q("a.."), q("json..x")]),
        None,
    );
}

#[test]
fn slash_backslash_segments_and_traversal() {
    common::interpreter_gate();
    // Segments containing path separators: pre-guard these joined/escaped
    // sys.path dirs ('/etc/..' imported /etc as a namespace pkg). CPython
    // reports the name truncated at the first offending separator.
    check_guard_case(
        "seps",
        &catch_probe(&[
            q("/"),
            q("//"),
            q("/os"),
            q("/etc/.."),
            q("os/"),
            q("os/.."),
            q("a/b/.."),
            q("./x"),
            q("a b"),
            q("a\\b"),
            q("\\a"),
            q("os\\.x"),
            q("json/../../etc"),
        ]),
        None,
    );
}

#[test]
fn name_repr_escaping_byte_exact() {
    common::interpreter_gate();
    // CPython embeds repr(name) in ModuleNotFoundError: backslash doubled,
    // quote-containing names switch to double quotes. Built with chr() so
    // identical source bytes reach both interpreters.
    check_guard_case(
        "repr",
        "def probe(name):\n    try:\n        __import__(name)\n        print('OK')\n    except BaseException as e:\n        print(type(e).__name__ + ': ' + str(e))\nprobe('a' + chr(92) + 'b')\nprobe(\"a\" + chr(39) + \"q\")\nprobe('a' + chr(92) + 'b.c')\n",
        None,
    );
}

// -------------------------------------------------------------- fixtures --

/// Unique per-process fixture root (tests run in parallel).
fn fixture_root(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("ap_import_guard_fix_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(dir.join("pkg")).expect("mkdir fixture");
    std::fs::write(dir.join("pkg/__init__.py"), "VALUE = 42\n")
        .expect("import_guard_differential.rs:270");
    std::fs::write(dir.join("pkg/mod.py"), "W = 7\n").expect("import_guard_differential.rs:271");
    std::fs::write(dir.join("mod.py"), "W = 7\n").expect("import_guard_differential.rs:272");
    dir
}

#[test]
fn traversal_under_real_package_dirs() {
    common::interpreter_gate();
    // With pkg/ on the path: 'pkg/..' must NOT resolve the sys.path parent
    // dir as a namespace package (pre-guard ap could walk out of sys.path
    // entries). CPython reports the name truncated at 'pkg/'.
    let root = fixture_root("trav");
    check_guard_case(
        "trav",
        &catch_probe(&[
            q("pkg/.."),
            q("pkg/../.."),
            q("pkg/../../etc"),
            q("pkg.mod/.."),
            q("pkg/."),
            q("pkg..x"),
            q("pkg./x"),
            q("pkg\\.x"),
        ]),
        Some(&root),
    );
}

#[test]
fn valid_imports_still_resolve() {
    common::interpreter_gate();
    // The guard must not change any parser-reachable import.
    let root = fixture_root("valid");
    let src = "\
import sys
def probe(name):
    try:
        __import__(name)
        print('OK', name in sys.modules)
    except BaseException as e:
        print(type(e).__name__ + ': ' + str(e))
probe('os.path')
probe('json')
probe('sys')
probe('builtins')
probe('keyword')
probe('linecache')
probe('pkg')
probe('pkg.mod')
import pkg.mod
print(pkg.mod.W)
from pkg import mod
print(mod.W)
import sys
print('os.path' in sys.modules, 'pkg' in sys.modules)
";
    check_guard_case("valid", src, Some(&root));
}

#[test]
fn uncaught_traceback_tail_and_exit_code() {
    common::interpreter_gate();
    // Exception-line bytes + exit status for the two headline shapes,
    // matching the import_differential.rs precedent (stderr LAST line).
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return,
    };
    for src in ["__import__('')\n", "__import__('..')\n", "__import__('')\n"] {
        let tmp = write_case("unc", src);
        let expected = Command::new(py)
            .arg("-S")
            .arg(&tmp)
            .output()
            .expect("spawn ref");
        let actual = Command::new(ap_run_eval())
            .arg(&tmp)
            .output()
            .expect("spawn ap");
        std::fs::remove_file(&tmp).ok();
        assert_eq!(expected.status.code(), actual.status.code(), "exit: {src}");
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
        assert_eq!(exp_last, act_last, "stderr tail mismatch for {src:?}");
    }
}
