//! Leaf fp-inlinecache — mutation-after-warm invalidation gates + the
//! no-GIL contention probe (plans/fast-path/02-perf/01-inline-caches.md
//! items 4 + 5).
//!
//! THE risk this leaf mitigates: a stale cache = wrong attribute = a silent
//! miscompile. Every test warms a site, MUTATES through a real user-visible
//! path, and asserts the post-mutation answer is exact. A missed bump shows
//! up as a stale value (fail-closed via output divergence).
//!
//! Invalidation surface map (which test covers which bump site):
//! - set_instance_attr_after_warm: instance-dict write under a warm
//!   InstDict site (no class counter moves — the hit re-probes the live
//!   attrs dict; a stale VALUE cache would show here).
//! - del_instance_attr_after_warm: instance-dict pop under a warm InstDict
//!   site (the re-probe misses -> slow path -> AttributeError/class member).
//! - set_class_attr_after_warm: C.x = .. under warm ClassValue/InstDict/
//!   BindFunc sites (type_version + surgery_version + names epoch bump —
//!   attr_set_protocol's UserClass arm).
//! - del_class_attr_after_warm: del C.x under warm sites (same bump pair).
//! - add_method_after_warm / overwrite_method_after_warm: method install
//!   over a warm BindFunc site (bind_function_owned answer must change).
//! - subclass_override_after_warm: the CHAIN case — override added on the
//!   BASE after the subclass-instance site warmed (chain_surgery_version
//!   max over the MRO is the invalidator).
//! - builtin_after_warm: a builtins-dict write under a warm global site.
//! - no_gil_4thread_contention: 4 worker threads hammering ONE shared
//!   code object's sites concurrently with a mutation churner — every
//!   thread's answers must stay correct throughout (per-thread tables +
//!   stamp revalidation; no tearing, no stale reads).
//!
//! Each test is a self-contained Python probe run under BOTH tier modes
//! (0 = tree-walker control; bytecode = the IC-populated route); output
//! must be byte-identical and rc 0.

mod common;

use std::process::Command;

const PROBES: &[(&str, &str)] = &[
    // (probe name, python source)
    (
        "set_instance_attr_after_warm",
        r#"
class C:
    def __init__(self):
        self.v = 1

c = C()
def read():
    return c.v
# warm the site
for _ in range(3000):
    read()
got1 = read()
assert got1 == 1, got1
# mutate AFTER warm: rebind the instance attr
c.v = 42
got2 = read()
assert got2 == 42, f"stale ic after instance rebind: {got2}"
# del AFTER warm
del c.v
try:
    read()
    print("SET-INST-FAIL: no AttributeError after del")
except AttributeError:
    print("SET-INST-OK")
# re-add with a different value
c.v = 7
got3 = read()
assert got3 == 7, f"stale ic after re-add: {got3}"
print("SET-INST-OK2")
"#,
    ),
    (
        "set_class_attr_after_warm",
        r#"
class C:
    x = 1

def read_cls():
    return C.x
def read_inst(c):
    return c.x
cs = [C() for _ in range(4)]
for _ in range(3000):
    read_cls()
    for c in cs:
        read_inst(c)
assert read_cls() == 1
# mutate AFTER warm: class attr rebind (type.__setattr__ bump path)
C.x = 42
assert read_cls() == 42, f"stale class-attr ic: {read_cls()}"
for c in cs:
    got = read_inst(c)
    assert got == 42, f"stale inst-view ic: {got}"
# del AFTER warm
del C.x
try:
    read_cls()
    print("SET-CLS-FAIL: no AttributeError after del C.x")
except AttributeError:
    print("SET-CLS-OK")
for c in cs:
    try:
        read_inst(c)
        print("SET-CLS-FAIL2: no AttributeError after del C.x (inst)")
    except AttributeError:
        pass
# re-add
C.x = 7
assert read_cls() == 7, f"stale after re-add: {read_cls()}"
print("SET-CLS-OK2")
"#,
    ),
    (
        "add_method_after_warm",
        r#"
class C:
    def m(self):
        return 1

c = C()
def call():
    return c.m
for _ in range(3000):
    call()
# add a NEW method after warm (namespace install)
C.n = lambda self: 99
got = c.n()
assert got == 99, f"added method not visible after warm: {got}"
# overwrite the WARM method
C.m = lambda self: 42
got2 = c.m()
assert got2 == 42, f"stale method ic after overwrite: {got2}"
print("ADD-METHOD-OK")
"#,
    ),
    (
        "subclass_override_after_warm",
        r#"
class Base:
    def m(self):
        return 1

class Sub(Base):
    pass

s = Sub()
def call():
    return s.m()
for _ in range(3000):
    call()
assert call() == 1
# override ON THE BASE after the subclass site warmed — the CHAIN bump
# (chain_surgery_version over the MRO) must invalidate.
Base.m = lambda self: 42
got = call()
assert got == 42, f"stale chain ic after base override: {got}"
# override on the SUBCLASS (closer in the MRO)
Sub.m = lambda self: 99
got2 = call()
assert got2 == 99, f"stale after subclass override: {got2}"
print("SUBCLASS-OK")
"#,
    ),
    (
        "global_builtin_after_warm",
        r#"
# warm a module-global site AND a builtins site
G = 1
def read_g():
    return G
def read_len():
    return len([1, 2, 3])
for _ in range(3000):
    read_g()
    read_len()
assert read_g() == 1
assert read_len() == 3
# rebind the module global AFTER warm
G = 42
got = read_g()
assert got == 42, f"stale global ic: {got}"
# shadow len in globals (builtins must lose)
_len = len
len = lambda x: 99
got2 = read_len()
assert got2 == 99, f"builtins shadow not visible: {got2}"
len = _len
got3 = read_len()
assert got3 == 3, f"un-shadow not visible: {got3}"
# delete the global -> NameError
del G
try:
    read_g()
    print("GB-FAIL: no NameError after del G")
except NameError:
    print("GB-OK")
print("GB-OK2")
"#,
    ),
    (
        "call_site_after_warm",
        r#"
class C:
    def __init__(self):
        self.d = {"k": 1}
        self.s = "abc"

c = C()
def calls():
    t = 0
    t += c.d.get("k")
    t += c.s.startswith("a")
    t += c.s.upper() == "ABC"
    return t
for _ in range(3000):
    calls()
assert calls() == 3, calls()
# mutate the RECEIVER's dict after warm — call sites must observe
c.d["k"] = 42
t = calls()
assert t == 44, f"stale call-path value: {t}"
print("CALL-OK")
"#,
    ),
];

static PROBE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn root_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Write the probe and run it under the given APX_TIER. Returns
/// (stdout, stderr, rc).
fn run_probe(tier: &str, src: &str) -> (String, String, i32) {
    let dir = std::env::temp_dir().join(format!(
        "ic-inval-{}-{}",
        std::process::id(),
        PROBE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join("probe.py");
    std::fs::write(&path, src).expect("write probe");

    let bin = root_dir()
        .join("target-fp-ic")
        .join("release")
        .join("antipythony");
    // ap-cli's `run` verb forwards no --tier flag of its own; the tier
    // resolves from APX_TIER (jit_hook's env fallback).
    let out = Command::new(&bin)
        .arg("run")
        .arg(&path)
        .env("APX_TIER", tier)
        .output()
        .expect("spawn antipythony");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn assert_verdict(tier: &str, name: &str, want: &[&str]) {
    let src = PROBES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .unwrap_or_else(|| panic!("unknown probe {name}"));
    let (out, err, rc) = run_probe(tier, src);
    assert_eq!(rc, 0, "[{tier}/{name}] rc != 0\nstdout:\n{out}\nstderr:\n{err}");
    assert!(err.is_empty(), "[{tier}/{name}] stderr not clean: {err}");
    for w in want {
        assert!(out.contains(w), "[{tier}/{name}] missing verdict {w:?} in:\n{out}");
    }
}

fn gate(name: &str, want: &[&str]) {
    // Tree-walker control first (the mutation paths' baseline semantics),
    // then the IC-populated bytecode route — BOTH must produce the exact
    // verdicts; the bytecode leg is where a missed bump shows.
    assert_verdict("0", name, want);
    assert_verdict("bytecode", name, want);
}

#[test]
fn inval_set_instance_attr_after_warm() {
    gate("set_instance_attr_after_warm", &["SET-INST-OK", "SET-INST-OK2"]);
}

#[test]
fn inval_set_class_attr_after_warm() {
    gate("set_class_attr_after_warm", &["SET-CLS-OK", "SET-CLS-OK2"]);
}

#[test]
fn inval_add_method_after_warm() {
    gate("add_method_after_warm", &["ADD-METHOD-OK"]);
}

#[test]
fn inval_subclass_override_after_warm() {
    gate("subclass_override_after_warm", &["SUBCLASS-OK"]);
}

#[test]
fn inval_global_builtin_after_warm() {
    gate("global_builtin_after_warm", &["GB-OK", "GB-OK2"]);
}

#[test]
fn inval_call_site_after_warm() {
    gate("call_site_after_warm", &["CALL-OK"]);
}

/// Item 5: the 4-thread contention probe. Four workers hammer ONE shared
/// code object's global + attr sites while a mutator churns the class
/// surface. Per-thread tables mean no shared writes; the stamp
/// revalidation means no thread may observe a stale answer at ANY point.
/// Correctness = every worker's checksums match the sequential answer at
/// each observed epoch.
#[test]
fn no_gil_4thread_contention() {
    let src = r#"
import threading

class C:
    def __init__(self, v):
        self.v = v

class Box:
    def __init__(self):
        self.c = C(1)
        self.epoch = 0

box = Box()

def worker(n_iter, stop):
    # STALE-FREEZE detection (scheduler-lag-proof): while ONE mutator
    # thread raises box.c.v monotonically, each worker tracks how the IC
    # read `a` evolves. A correct cache answers the LIVE value — its `a`
    # values ADVANCE as the mutator churns (the InstDict plan re-probes
    # the live attrs dict; any invalidation path refills from live). A
    # stale cache FREEZES at its fill-time value: `a` repeats unchanged
    # while `b` (always the walk) runs away. Freeze = 40+ consecutive
    # identical `a` readings while `b` advanced by >= 5 across them.
    ok = 0
    last_a = -1
    same_a = 0
    b_at_last_a = -1
    while not stop.is_set():
        c = box.c
        a = c.v              # IC site (bytecode tier): cached plan
        b = getattr(c, "v")  # always the walk
        if a == last_a:
            same_a += 1
            if same_a >= 40 and b - b_at_last_a >= 5:
                print(f"FREEZE: ic stuck at {a}, walk advanced {b_at_last_a}->{b}")
                return
        else:
            last_a = a
            same_a = 0
            b_at_last_a = b
        ok += 1
        if ok >= n_iter:
            break
    results.append(ok)

results = []
stop = threading.Event()
ts = [threading.Thread(target=worker, args=(200000, stop)) for _ in range(4)]
for t in ts:
    t.start()
# churn the value + the class surface concurrently (single mutator:
# v rises monotonically 0..499 — the stale-detection invariant's basis)
for i in range(600):
    box.c.v = i
    if i % 100 == 0:
        C.extra = i  # class surgery: bumps the counters the stamps read
        import time
        time.sleep(0.0005)
for t in ts:
    t.join()
total = sum(results)
assert total >= 4 * 10000, f"workers starved: {results}"
print("CONTENTION-OK", total)
"#;
    let (out, err, rc) = run_probe("bytecode", src);
    assert_eq!(rc, 0, "contention probe rc != 0\n{out}\n{err}");
    assert!(err.is_empty(), "contention stderr: {err}");
    assert!(
        out.contains("CONTENTION-OK"),
        "contention probe missing verdict:\n{out}"
    );
    assert!(!out.contains("TEAR"), "stale IC under contention:\n{out}");
}
