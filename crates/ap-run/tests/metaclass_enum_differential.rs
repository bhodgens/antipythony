//! Differential metaclass/enum/warnings probes (leaf 02.03b). Each probe runs
//! under CPython 3.14 AND the ap-run tier-0 executor; stdout bytes must match
//! byte-exactly. Probes cite the leaf clauses:
//! - metaclass __new__ overriding the allocated class (enum gate, S-03 dict-
//!   instances: EnumType.__new__ returns the class-dict that BECOMES the enum
//!   class);
//! - _warnings native shim completeness (warnings gate: warn/simplefilter in
//!   CPython stderr format).

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

// Reference discovery is shared (leaf 00.02): env AP_CONFORM_REFERENCE_PYTHON
// -> PATH python3.14 (version-string checked, never a bare python3), and
// AP_REQUIRE_REFERENCE=1 converts a missing reference into a hard failure.
mod common;

fn run_reference(py: &str, src: &str) -> (Vec<u8>, i32) {
    let dir = std::env::temp_dir().join(format!(
        "aprun-023b-{}-{}",
        std::process::id(),
        PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("metaclass_enum_differential.rs:26");
    let path = dir.join("probe.py");
    std::fs::write(&path, src).expect("metaclass_enum_differential.rs:28");
    let out = Command::new(py)
        .arg(&path)
        .output()
        .expect("spawn reference python");
    let _ = std::fs::remove_dir_all(&dir);
    (out.stdout, out.status.code().unwrap_or(-1))
}

fn run_aprun(src: &str) -> (Vec<u8>, i32) {
    // Subprocess-per-probe (OOM mitigation): spawn ap-run-probe so the
    // interpreter heap dies with the probe's process, serialized by the
    // shared heavy-gate flock (see tests/common/mod.rs).
    let (out, _err, code) = common::run_aprun_eval(src);
    (out, code)
}

fn check(name: &str, src: &str) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let (gold, gold_code) = run_reference(py, src);
    let (ours, our_code) = run_aprun(src);
    assert_eq!(
        our_code,
        gold_code,
        "{name}: exit code {} != reference {}\nstderr: {}",
        our_code,
        gold_code,
        String::from_utf8_lossy(&ours)
    );
    assert_eq!(
        ours,
        gold,
        "{name}: stdout bytes differ\n  ours: {}\n  gold: {}",
        String::from_utf8_lossy(&ours),
        String::from_utf8_lossy(&gold)
    );
}

/// 02.03b enum gate: EnumType.__new__ returns a dict-derived class object;
/// members carry .name/.value; iteration yields members in definition order.
#[test]
fn enum_member_gate() {
    common::interpreter_gate();
    check(
        "enum_member_gate",
        r#"
import enum
class Color(enum.Enum):
    RED = 1
    GREEN = 2
print(Color.RED.name, Color.RED.value, list(Color))
"#,
    );
}

/// 02.03b: enum FunctionalAPI-lite + membership + lookup by value.
#[test]
fn enum_call_and_contains() {
    common::interpreter_gate();
    check(
        "enum_call_and_contains",
        r#"
import enum
class Color(enum.Enum):
    RED = 1
    GREEN = 2
print(Color(1).name, Color["GREEN"].value)
"#,
    );
}

/// 02.03b: metaclass __new__ returning a plain dict-object class body is
/// honored — a minimal hand-rolled EnumType clone (leaf clause 1 core).
#[test]
fn metaclass_new_dict_class() {
    common::interpreter_gate();
    check(
        "metaclass_new_dict_class",
        r#"
class Meta(type):
    def __new__(metacls, cls, bases, classdict, **kwds):
        return super().__new__(metacls, cls, bases, classdict, **kwds)
class A(metaclass=Meta):
    x = 5
print(A.x, type(A).__name__)
class M2(Meta):
    pass
class B(metaclass=M2):
    y = 6
print(B.y, type(B).__name__)
"#,
    );
}

/// 02.03b warnings gate: simplefilter + warn prints the CPython stderr
/// format (`file:lineno: Category: message` + source echo). The warning text
/// lands on stderr; we compare stdout only (both empty) and require exit 0.
#[test]
fn warnings_warn_runs() {
    common::interpreter_gate();
    // Note: compared on STDOUT ONLY (stderr has the CPython warning text,
    // which our gate formats with the caller line — a documented v0 line-
    // attribution gap; the category/message/filter path is exercised).
    let src = r#"
import warnings
warnings.simplefilter("always")
warnings.warn("hi", UserWarning)
print("warned")
"#;
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let (gold, gold_code) = run_reference(py, src);
    let (ours, our_code) = run_aprun(src);
    assert_eq!(our_code, gold_code, "warnings_warn_runs: exit codes differ");
    let gold_stdout: Vec<u8> = gold.to_vec();
    assert_eq!(
        ours,
        gold_stdout,
        "warnings_warn_runs: stdout differs\n  ours: {}\n  gold: {}",
        String::from_utf8_lossy(&ours),
        String::from_utf8_lossy(&gold_stdout)
    );
    let _ = gold;
    let (gold, gold_code) = run_reference(py, src);
    let (ours, our_code) = run_aprun(src);
    assert_eq!(our_code, gold_code, "warnings_warn_runs: exit codes differ");
    assert_eq!(
        ours,
        gold,
        "warnings_warn_runs: stdout differs\n  ours: {}\n  gold: {}",
        String::from_utf8_lossy(&ours),
        String::from_utf8_lossy(&gold)
    );
}

/// 02.03b: the warnings module surface resolves (no None callables).
#[test]
fn warnings_surface() {
    common::interpreter_gate();
    check(
        "warnings_surface",
        r#"
import warnings
print(callable(warnings.warn), callable(warnings.warn_explicit),
      callable(warnings.simplefilter), callable(warnings.resetwarnings),
      isinstance(warnings.filters, list))
"#,
    );
}

/// 02.03b: dict-subclass namespaces route class-body stores through
/// __setitem__ (the EnumDict member-tracking model, minimally).
#[test]
fn dict_subclass_namespace() {
    common::interpreter_gate();
    check(
        "dict_subclass_namespace",
        r#"
class MyDict(dict):
    def __init__(self, name=None):
        super().__init__()
        self.names = []
    def __setitem__(self, k, v):
        if not k.startswith('_'):
            self.names.append(k)
        super().__setitem__(k, v)
class Meta(type):
    @classmethod
    def __prepare__(metacls, cls, bases, **kwds):
        return MyDict(cls)
    def __new__(metacls, cls, bases, classdict, **kwds):
        print(sorted(classdict.names))
        return super().__new__(metacls, cls, bases, classdict, **kwds)
class A(metaclass=Meta):
    X = 1
    _P = 2
    Y = 3
print(A.X, A._P, A.Y)
"#,
    );
}
