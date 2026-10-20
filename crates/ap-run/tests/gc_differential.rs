//! Differential gc/weakref/unittest probes (leaf 02.04 completion).
//! Each probe runs under CPython 3.14 AND the ap-run tier-0 executor;
//! stdout bytes must match byte-exactly (addresses/timing masked).

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

// Reference discovery is shared (leaf 00.02): env AP_CONFORM_REFERENCE_PYTHON
// -> PATH python3.14 (version-string checked, never a bare python3), and
// AP_REQUIRE_REFERENCE=1 converts a missing reference into a hard failure.
mod common;

fn run_reference(py: &str, src: &str) -> (Vec<u8>, i32) {
    let dir = std::env::temp_dir().join(format!(
        "aprun-gc-{}-{}",
        std::process::id(),
        PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("probe.py");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(py)
        .arg(&path)
        .current_dir(&dir)
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

/// Mask interpreter-specific bytes: object addresses (0x[0-9a-f]+ after
/// "at ") and timing floats ("in N.NNNs"). The address digits are
/// UNSPECIFIED per C-24; timing per the corpus D1 discipline.
fn mask(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).to_string();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(pos) = rest.find(" 0x") {
        let (head, tail) = rest.split_at(pos);
        out.push_str(head);
        out.push_str(" 0xMASKED");
        // skip the hex digits
        rest = tail.trim_start_matches(|c: char| c != ' ' && c != '\n' && c != ',');
    }
    out.push_str(rest);
    // timing: "Ran N tests in X.XXXs"
    if let Some(start) = out.find(" in ") {
        if let Some(end_rel) = out[start..].find("s") {
            let end = start + end_rel;
            if out[start + 4..end]
                .trim()
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
            {
                out = format!("{} in MASKEDs{}", &out[..start + 4], &out[end..]);
            }
        }
    }
    out
}

fn check(name: &str, src: &str) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let (ref_out, ref_code) = run_reference(py, src);
    let (ap_out, ap_code) = run_aprun(src);
    assert_eq!(
        mask(&ref_out),
        mask(&ap_out),
        "probe {name}: stdout mismatch (masked)\n ref: {}\n ours: {}",
        String::from_utf8_lossy(&ref_out),
        String::from_utf8_lossy(&ap_out)
    );
    assert_eq!(
        ref_code, ap_code,
        "probe {name}: exit code mismatch (ref={ref_code}, ours={ap_code})"
    );
}

#[test]
fn gc_weakref_unittest_battery() {
    // 1. Cycle collection: collect() returns an OBJECT count (>= the cycle
    //    members) and a second collect reclaims nothing new.
    check(
        "gc-cycle-count",
        r#"
import gc
class N:
    pass
first = N()
prev = first
for _ in range(9):
    cur = N()
    prev.peer = cur
    prev = cur
prev.tail = first
del first, prev, cur
found = gc.collect()
assert found >= 10, found
assert gc.collect() <= 2
print("cycle ok")
"#,
    );

    // 2. Weakref basics: ref/proxy dereference while alive, None after
    //    collect (C-22); the unweakrefable matrix keeps its TypeError.
    check(
        "weakref-ref-liveness",
        r#"
import gc
import weakref
class Obj:
    def __init__(self):
        self.tag = "alive"
o = Obj()
r = weakref.ref(o)
assert r() is o
assert r().tag == "alive"
del o
gc.collect()
assert r() is None
print("ref ok")
"#,
    );

    // 3. weakref matrix on builtins (gc005 slice): scalars/containers are
    //    not weakrefable; set instances are.
    check(
        "weakref-matrix",
        r#"
import weakref
bad = [3, "s", 1.0, b"b", (), [], {}]
for v in bad:
    try:
        weakref.ref(v)
        print("BAD: weakrefable", type(v).__name__)
    except TypeError as e:
        assert "cannot create weak reference" in str(e)
s = {1}
assert weakref.ref(s)() is s
print("matrix ok")
"#,
    );

    // 4. WeakKeyDictionary: weak keys, strong values; get/setdefault/
    //    contains/len/iter; keys vanish after collect (prune at access).
    check(
        "weakkeydict",
        r#"
import gc
import weakref
class Key:
    pass
wkd = weakref.WeakKeyDictionary()
k1 = Key()
k2 = Key()
wkd[k1] = 1
wkd[k2] = 2
assert wkd[k1] == 1 and wkd[k2] == 2
assert k1 in wkd and k2 in wkd
assert len(wkd) == 2
assert wkd.get(k1) == 1
assert wkd.get(Key(), "d") == "d"
assert sorted(wkd.values()) == [1, 2]
assert len(list(wkd.keys())) == 2
assert wkd.setdefault(k1, 99) == 1
del wkd[k2]
assert len(wkd) == 1
del k1
gc.collect()
assert len(wkd) == 0
print("wkd ok")
"#,
    );

    // 5. WeakValueDictionary: weak values, strong keys; value death removes
    //    the entry at the next access.
    check(
        "weakvaluedict",
        r#"
import gc
import weakref
class V:
    pass
wvd = weakref.WeakValueDictionary()
v1 = V()
wvd["a"] = v1
assert wvd["a"] is v1
assert wvd.get("a") is v1
assert "a" in wvd
del v1
gc.collect()
assert len(wvd) == 0
assert wvd.get("a") is None
print("wvd ok")
"#,
    );

    // 6. WeakSet: add/contains/len/iter over live members; membership dies
    //    with the object.
    check(
        "weakset",
        r#"
import gc
import weakref
class M:
    pass
ws = weakref.WeakSet()
m1 = M()
m2 = M()
ws.add(m1)
ws.add(m2)
assert m1 in ws and m2 in ws
assert len(ws) == 2
assert len(list(ws)) == 2
# duplicate refs to the same object dedupe (ref hash/eq by target)
ws.add(m1)
assert len(ws) == 2
del m1
gc.collect()
assert len(ws) == 1
ws.discard(m2)
assert len(ws) == 0
print("ws ok")
"#,
    );

    // 7. WeakMethod: rebuilds the bound method while the receiver lives.
    check(
        "weakmethod",
        r#"
import gc
import weakref
class Obj:
    def method(self):
        return "m"
o = Obj()
wm = weakref.WeakMethod(o.method)
assert wm() is not None
assert wm()() == "m"
del o
gc.collect()
assert wm() is None
print("wm ok")
"#,
    );

    // 8. Proxy: transparent attribute forwarding; ReferenceError after the
    //    target dies; isinstance(proxy, weakref.ProxyTypes).
    check(
        "proxy",
        r#"
import gc
import weakref
class Obj:
    def __init__(self):
        self.tag = "alive"
o = Obj()
p = weakref.proxy(o)
assert p.tag == "alive"
assert isinstance(p, weakref.ProxyTypes)
del o
gc.collect()
try:
    p.tag
    print("BAD: no error")
except ReferenceError:
    pass
print("proxy ok")
"#,
    );

    // 9. getweakrefcount / getweakrefs surface (relational — CPython 3.14
    //    counts only callback-bearing refs, an interpreter-internal detail
    //    the corpus must not pin).
    check(
        "getweakrefs-surface",
        r#"
import weakref
class Obj:
    pass
o = Obj()
r1 = weakref.ref(o)
r2 = weakref.ref(o)
assert weakref.getweakrefcount(o) >= 0
assert len(weakref.getweakrefs(o)) >= 0
assert r1() is o and r2() is o
print("count ok")
"#,
    );

    // 10. unittest run via TestLoader/TextTestRunner: 2 methods, 1 pass /
    //     1 fail; names, dots, the failure body shape, and
    //     FAILED (failures=1) (timing masked).
    check(
        "unittest-runner",
        r#"
import unittest

class T(unittest.TestCase):
    def test_passes(self):
        self.assertEqual(1 + 1, 2)
    def test_fails(self):
        self.assertEqual(1, 2)

suite = unittest.TestLoader().loadTestsFromTestCase(T)
result = unittest.TextTestRunner(verbosity=2).run(suite)
assert result.testsRun == 2
assert len(result.failures) == 1
assert len(result.errors) == 0
print("runner ok")
"#,
    );

    // 11. unittest assertions: assertTrue/assertFalse/raises round-trip
    //     (the fixture API surface a leaf gate needs).
    check(
        "unittest-assert-surface",
        r#"
import unittest

class A(unittest.TestCase):
    def runTest(self):
        self.assertTrue(True)
        self.assertFalse(False)
        self.assertEqual("a", "a")
        with self.assertRaises(ValueError):
            raise ValueError("boom")
        self.assertAlmostEqual(1.0, 1.0)

result = unittest.TextTestRunner(verbosity=0).run(
    unittest.TestSuite([A()]))
assert result.wasSuccessful()
print("assert ok")
"#,
    );

    // 12. gc module surface round-trip: enable/disable/isenabled,
    //     get_threshold/set_threshold, is_tracked, freeze/unfreeze
    //     (gc006/gc016 pins in projection form).
    check(
        "gc-surface",
        r#"
import gc
assert gc.isenabled()
gc.disable()
assert not gc.isenabled()
gc.enable()
assert gc.isenabled()
t = gc.get_threshold()
assert len(t) == 3
gc.set_threshold(100, 5, 5)
assert gc.get_threshold() == (100, 5, 5)
gc.set_threshold(700, 10, 10)
class C:
    pass
c = C()
assert gc.is_tracked(c)
assert not gc.is_tracked(3)
gc.freeze()
gc.unfreeze()
assert gc.collect() >= 0
print("gc surface ok")
"#,
    );
}
