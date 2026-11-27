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
        "instance_shadow_after_warm",
        r#"
class C:
    x = 1

c = C()

def read():
    return c.x

for _ in range(3000):
    a = read()
assert a == 1, a
print("SHADOW-X-WARM", a)
# Instance write shadows the warm ClassValue site: no class stamp moves.
c.x = 42
got = read()
assert got == 42, f"stale class-value ic after instance shadow: {got}"
print("SHADOW-X-OK", got)
"#,
    ),
    (
        "method_shadow_after_warm",
        r#"
class C:
    def m(self):
        return "class-method"

c = C()

def call():
    f = c.m
    return f()

for _ in range(3000):
    b = call()
assert b == "class-method", b
print("SHADOW-M-WARM", b)
# Instance attr shadows the warm BindFunc site: no class stamp moves.
c.m = lambda: "instance-lambda"
got = call()
assert got == "instance-lambda", f"stale bindfunc ic after instance shadow: {got}"
print("SHADOW-M-OK", got)
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
        .join("target-fp-ic2b")
        .join("release")
        .join("antipythony");
    // ap-cli's `run` verb forwards no --tier flag of its own; the tier
    // resolves from APX_TIER (jit_hook's env fallback). Strip ANY inherited
    // APX_* diagnostic env first (a leaked APX_TIER_STATS/APX_TIER in the
    // cargo runner once produced false REDs), then set the one under test.
    let mut cmd = Command::new(&bin);
    for (k, _) in std::env::vars() {
        if k.starts_with("APX_") {
            cmd.env_remove(&k);
        }
    }
    let out = cmd
        .arg("run")
        .arg(&path)
        .env("APX_TIER", tier)
        .env("APX_TIER_STATS", "1") // the ahit proof needs the stats line
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
    // [TIER-STATS]/[BC-STATS] transparency lines are expected on stderr
    // (fp-exceptions' stats emitter prints them unconditionally on the
    // bytecode route); filter them before the clean-stderr assert.
    let noisy = err
        .lines()
        .filter(|l| !l.trim_start().starts_with("[TIER-STATS]"))
        .filter(|l| !l.trim_start().starts_with("[BC-STATS]"))
        .count();
    assert!(
        noisy == 0,
        "[{tier}/{name}] stderr not clean: {err}"
    );
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

/// Extract `ahit=N` from the [BC-STATS] line. Panics when absent so a
/// route regression (silent tree-walk fallback) can't pass vacuously.
fn bc_ahit(err: &str) -> u64 {
    let line = err
        .lines()
        .find(|l| l.trim_start().starts_with("[BC-STATS]"))
        .unwrap_or_else(|| panic!("missing [BC-STATS] line in stderr: {err}"));
    let tail = line.split("ahit=").nth(1).expect("ahit field");
    tail.split_whitespace()
        .next()
        .unwrap()
        .parse()
        .expect("ahit integer")
}

fn gate_with_ic_hits(name: &str, want: &[&str]) {
    // Tree-walker control first (exact verdict), then the bytecode route:
    // both must produce the verdicts AND the bytecode leg must prove a
    // genuine inline-cache hit (ahit > 0) — otherwise this gate would pass
    // vacuously by silently falling back to the tree-walker.
    assert_verdict("0", name, want);
    let src = PROBES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .unwrap_or_else(|| panic!("unknown probe {name}"))
        .to_string();
    let (out, err, rc) = run_probe("bytecode", &src);
    assert_eq!(rc, 0, "[bytecode/{name}] rc != 0\n{out}\n{err}");
    let ahit = bc_ahit(&err);
    assert!(
        ahit > 0,
        "[bytecode/{name}] ic hit arm never exercised (ahit={ahit}):\n{err}"
    );
    for w in want {
        assert!(out.contains(w), "[bytecode/{name}] missing verdict {w:?} in:\n{out}");
    }
}

#[test]
fn inval_instance_shadow_after_warm() {
    gate_with_ic_hits(
        "instance_shadow_after_warm",
        &["SHADOW-X-WARM 1", "SHADOW-X-OK 42"],
    );
}

#[test]
fn inval_method_shadow_after_warm() {
    gate_with_ic_hits(
        "method_shadow_after_warm",
        &["SHADOW-M-WARM class-method", "SHADOW-M-OK instance-lambda"],
    );
}

/// Item 5: the concurrent-mutation probe. Four worker threads hammer the
/// SAME instance dict while the MAIN thread hot-reads that dict through a
/// warmed bytecode IC site. Worker fibers run in tree tier (TIER_MODE is
/// thread-local), so the IC hit arm is exercised on the main thread only;
/// ahit>0 in [BC-STATS] proves the gate is not vacuous. Correctness =
/// monotonic reads, strict per-worker counts, and a final read equal to
/// the last concurrent write.
#[test]
fn no_gil_4thread_contention() {
    let src = r#"
import threading

class C:
    def __init__(self, v):
        self.v = v

c = C(0)

def read():
    return c.v

# Warm the IC site on the MAIN thread (the only tier-aware thread: worker
# fibers get the tree tier). ahit>0 proves the bytecode hit arm ran.
for _ in range(2000):
    a = read()
assert a == 0, a

N = 200000
stop = [False]
counts = [0, 0, 0, 0]

def mutator(idx):
    ok = 0
    for i in range(N):
        c.v = i
        ok += 1
    counts[idx] = ok

ts = [threading.Thread(target=mutator, args=(i,)) for i in range(4)]
for t in ts:
    t.start()
# MAIN thread hot-reads while 4 workers mutate the SAME instance dict.
# A stale IC would freeze at a fill-time value while writes advance; we
# sample monotonic epochs and require both strict worker counts and a
# final read equal to the last write.
samples = 0
while not all(not t.is_alive() for t in ts):
    v = read()
    if v < 0 or v >= N:
        print(f"FREEZE OUT-OF-RANGE: {v}")
        break
    samples += 1
for t in ts:
    t.join()
for n in counts:
    assert n == N, f"mutator count {n} != {N}: {counts}"
assert samples > 0, "main thread never sampled a concurrent read"
final = read()
assert final == N - 1, f"stale final IC read: {final} != {N-1}"
print("CONTENTION-OK", samples, final)
"#;
    let (out, err, rc) = run_probe("bytecode", src);
    assert_eq!(rc, 0, "contention probe rc != 0\n{out}\n{err}");
    // Stats-transparency lines ([TIER-STATS]/[BC-STATS], unconditional on
    // the bytecode route since fp-exceptions) are expected noise.
    let noisy = err
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("[TIER-STATS]") || t.starts_with("[BC-STATS]"))
        })
        .count();
    assert!(noisy == 0, "contention stderr: {err}");
    assert!(
        out.contains("CONTENTION-OK"),
        "contention probe missing verdict:\n{out}"
    );
    // The probe emits "FREEZE: ..." (NOT "TEAR" — that typo made this
    // assertion vacuous). A stale IC under contention must fail here.
    assert!(
        !out.contains("FREEZE"),
        "stale IC under contention:\n{out}"
    );
    let ahit = bc_ahit(&err);
    assert!(ahit > 0, "contention never hit an IC site (ahit={ahit}):\n{err}");
}
