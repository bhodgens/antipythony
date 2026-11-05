//! Differential metaclass `__new__` arity probes (leaf 02.03b fix pass).
//! Each probe runs under CPython 3.14 AND the ap-run tier-0 executor;
//! stdout bytes must match byte-exactly. Root cause these fence:
//! CPython treats user `__new__` as an implicit staticmethod
//! (`super().__new__` resolves UNBOUND; callers pass cls/mcs explicitly),
//! while the runtime bound it like a normal method and doubled the
//! receiver — "TypeError: __new__() takes 4 positional arguments but 5
//! were given" on every `class KeyedRef(_weakref.ref)`-style chain.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

// Reference discovery is shared (leaf 00.02): env AP_CONFORM_REFERENCE_PYTHON
// -> PATH python3.14 (version-string checked, never a bare python3), and
// AP_REQUIRE_REFERENCE=1 converts a missing reference into a hard failure.
mod common;

fn run_reference(py: &str, src: &str) -> (Vec<u8>, i32) {
    let dir = std::env::temp_dir().join(format!(
        "aprun-meta-arity-{}-{}",
        std::process::id(),
        PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.py");
    std::fs::write(&path, src).unwrap();
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
    let (ref_out, ref_code) = run_reference(py, src);
    let (ap_out, ap_code) = run_aprun(src);
    assert_eq!(
        ref_out,
        ap_out,
        "probe {name}: stdout mismatch\n ref: {}\n ours: {}",
        String::from_utf8_lossy(&ref_out),
        String::from_utf8_lossy(&ap_out)
    );
    assert_eq!(
        ref_code, ap_code,
        "probe {name}: exit code mismatch (ref={ref_code}, ours={ap_code})"
    );
}

#[test]
fn metaclass_new_arity_battery() {
    // 1. Simple metaclass, zero-arg super() chain (0 bases).
    check(
        "meta-0base",
        r#"
class Meta(type):
    def __new__(mcs, name, bases, ns):
        print("Meta.__new__", name)
        return super().__new__(mcs, name, bases, ns)
class A(metaclass=Meta):
    pass
print("ok", type(A).__name__)
"#,
    );

    // 2. Two-level metaclass inheritance (B(A) re-triggers).
    check(
        "meta-2level",
        r#"
class Base(type):
    def __new__(mcs, name, bases, ns, **kw):
        f = super().__new__
        return f(mcs, name, bases, ns, **kw)
class M(Base):
    def __new__(cls, name, bases, ns, **kw):
        f = super().__new__
        return f(cls, name, bases, ns, **kw)
class A(metaclass=M):
    pass
print("ok", type(A).__name__)
"#,
    );

    // 3. **kw pass-through metaclass with class kwargs.
    check(
        "meta-kw",
        r#"
class Meta(type):
    def __new__(mcs, name, bases, ns, **kw):
        print("kw:", kw)
        return super().__new__(mcs, name, bases, ns, **kw)
class A(metaclass=Meta, foo=1):
    pass
print("done")
"#,
    );

    // 4. Instance __new__ chain with *args (object.__new__ terminal).
    check(
        "inst-new-chain",
        r#"
class C:
    def __new__(cls, *args):
        return super().__new__(cls)
class D(C):
    def __new__(cls, *args):
        return super().__new__(cls, *args)
d = D(5)
print("ok")
"#,
    );

    // 5. ABCMeta-style user metaclass __new__ (unbound super().__new__ with
    //    explicit mcls) + subclass re-trigger.
    check(
        "abc-style",
        r#"
from abc import ABCMeta
class M(ABCMeta):
    def __new__(cls, name, bases, ns, **kw):
        return super().__new__(cls, name, bases, ns, **kw)
class A(metaclass=M):
    pass
class B(A):
    pass
print("ok", type(A).__name__, type(B).__name__)
"#,
    );

    // 6. 3-base chain through a user metaclass.
    check(
        "meta-3base",
        r#"
class M(type):
    def __new__(mcs, name, bases, ns):
        return super().__new__(mcs, name, bases, ns)
class X: pass
class Y: pass
class Z: pass
class A(X, Y, metaclass=M):
    pass
class B(A, Z):
    pass
print("ok", len(B.__mro__))
"#,
    );

    // 7. direct type.__new__ (no super).
    check(
        "direct-type-new",
        r#"
class M(type):
    def __new__(mcs, name, bases, ns):
        return type.__new__(mcs, name, bases, ns)
class A(metaclass=M):
    pass
print("ok", type(A).__name__)
"#,
    );

    // 8. gates: unittest + pathlib imports and the pathlib reprs.
    check("gate-unittest", "import unittest\nprint('ok')\n");
    check(
        "gate-pathlib",
        "import pathlib\nprint(str(pathlib.Path('.')))\n",
    );
    check(
        "gate-pureposix-str",
        "import pathlib\nprint(str(pathlib.PurePosixPath('/a/b')))\n",
    );
    check(
        "gate-pureposix-repr",
        "import pathlib\nprint(repr(pathlib.PurePosixPath('/a/b')))\n",
    );
}
