//! Differential metaclass `__new__` arity probes (leaf 02.03b fix pass).
//! Each probe runs under CPython 3.14 AND the ap-run tier-0 executor;
//! stdout bytes must match byte-exactly. Root cause these fence:
//! CPython treats user `__new__` as an implicit staticmethod
//! (`super().__new__` resolves UNBOUND; callers pass cls/mcs explicitly),
//! while the runtime bound it like a normal method and doubled the
//! receiver — "TypeError: __new__() takes 4 positional arguments but 5
//! were given" on every `class KeyedRef(_weakref.ref)`-style chain.

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

fn ref_python() -> &'static str {
    for cand in ["/opt/homebrew/bin/python3.14", "python3.14", "python3"] {
        if let Ok(out) = Command::new(cand).arg("--version").output() {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            if out.status.success() && text.contains("Python 3.14") {
                return cand;
            }
        }
    }
    panic!("no reference python3.14 on this host");
}

fn run_reference(src: &str) -> (Vec<u8>, i32) {
    let dir = std::env::temp_dir().join(format!(
        "aprun-meta-arity-{}-{}",
        std::process::id(),
        PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.py");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(ref_python())
        .arg(&path)
        .output()
        .expect("spawn reference python");
    let _ = std::fs::remove_dir_all(&dir);
    (out.stdout, out.status.code().unwrap_or(-1))
}

fn run_aprun(src: &str) -> (Vec<u8>, i32) {
    let src = src.to_string();
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let mut out: Vec<u8> = Vec::new();
            let mut err: Vec<u8> = Vec::new();
            let outcome = ap_run::run_module(&src, "probe.py", &mut out, &mut err);
            let _ = Write::flush(&mut out);
            (out, outcome.exit_code)
        })
        .expect("spawn worker thread");
    handle.join().expect("worker panicked")
}

fn check(name: &str, src: &str) {
    let (ref_out, ref_code) = run_reference(src);
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
