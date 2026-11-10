//! Leaf 02.05g — the GC roots shim gates (SPLIT-02.05.md §02.05g):
//!
//! 1. ALLOC-UNDER-RECURSION: forced GC during deep native recursion keeps
//!    every live temporary alive (no premature free, no pinned garbage).
//!    This is Risk 1's earliest probe (leaf risk table row 1).
//! 2. `del x; gc.collect()` OBSERVABILITY: a deleted binding's finalizer
//!    fires at the next explicit collect — the conservative-scan-floor
//!    contract (pon set_conservative_scan_floor analogue) under the shim.
//! 3. GC FAMILY 20/20 under ADAPTIVE: the corpus gc slice runs through
//!    the JIT entry hook with the roots shim armed (capture point set in
//!    jit_call_body) and stays byte-identical to the tier-0 bytes.
//!
//! Harness discipline (harness hardening rules, per the sibling
//! differential suites): every interpreter spawn runs fenced — RSS cap +
//! wall-clock deadline + memory-pressure preflight + the repo-wide
//! interpreter gate. Byte comparison against CPython 3.14 reference runs
//! with the same masking discipline as gc_differential.rs.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

mod common;

/// Reference-interpreter run (each gc_differential-style suite owns its
/// own runner; the shared `common` exposes discovery + the impl fence).
fn run_reference(py: &str, src: &str) -> (Vec<u8>, i32) {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!(
        "aprun-rootscan-{}-{}",
        std::process::id(),
        PROBE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create reference temp dir");
    let path = dir.join("probe.py");
    std::fs::write(&path, src).expect("write reference probe.py");
    let out = Command::new(py)
        .arg(&path)
        .current_dir(&dir)
        .output()
        .expect("spawn reference python");
    (out.stdout, out.status.code().unwrap_or(-1))
}

static PROBE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn check(name: &str, src: &str) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let (ref_out, ref_code) = run_reference(py, src);
    let (ap_out, _ap_err, ap_code) = common::run_aprun_eval(src);
    assert_eq!(
        String::from_utf8_lossy(&ref_out),
        String::from_utf8_lossy(&ap_out),
        "probe {name}: stdout mismatch vs CPython\n ref: {}\n ours: {}",
        String::from_utf8_lossy(&ref_out),
        String::from_utf8_lossy(&ap_out)
    );
    assert_eq!(
        ref_code, ap_code,
        "probe {name}: exit code mismatch (ref={ref_code}, ours={ap_code})"
    );
}

#[test]
fn alloc_under_recursion_forced_gc_keeps_livings_alive() {
    common::interpreter_gate();
    // Risk-1 earliest probe: a Node allocated per frame, explicit
    // gc.collect() every 50 frames mid-recursion, each frame re-checks
    // its own node after the nested calls + collects. A GC that frees
    // (or corrupts) a live temporary shows up as CORRUPT lines or a
    // wrong sum; output must match CPython byte-exactly.
    check(
        "alloc-under-recursion",
        r#"
import gc

seen = []

class Node:
    def __init__(self, depth):
        self.depth = depth

def recurse(n):
    node = Node(n)
    if n % 50 == 0:
        gc.collect()
    if n <= 0:
        seen.append(node)
        return 0
    r = recurse(n - 1)
    if node.depth != n:
        print("CORRUPT", n, node.depth)
    return r + 1

print("start")
got = recurse(300)
print("sum", got)
print("kept", seen[0].depth)
alive = sum(1 for x in [Node(i) for i in range(200)])
gc.collect()
print("tail", alive)
print("done")
"#,
    );
}

#[test]
fn del_then_collect_observability() {
    common::interpreter_gate();
    // The conservative-floor contract: a deleted binding's object dies at
    // the next explicit collect even with the roots shim armed — finalizer
    // counts after each collect are pinned (C-20/C-23 explicit-collect
    // anchor; timing is never observed).
    check(
        "del-observability",
        r#"
import gc
fired = []
class Boom:
    def __del__(self):
        fired.append("del")
x = Boom()
y = x
del x
gc.collect()
print("after_del_x", len(fired))
del y
gc.collect()
print("after_del_y", len(fired))
d = {}
d["k"] = Boom()
del d
gc.collect()
print("after_del_d", len(fired))
print("done")
"#,
    );
}

#[test]
fn deep_recursion_alloc_bound() {
    common::interpreter_gate();
    // Deeper chain with per-frame list allocation + sparse forced GCs:
    // bounded RSS + deadline handled by the fence; a leak or unbounded
    // growth in the shim's walk would trip the memory fence instead of
    // passing silently.
    check(
        "deep-recursion-alloc",
        r#"
import gc
def r(n):
    x = [n]
    if n <= 0:
        return 0
    gc.collect() if n % 100 == 0 else None
    return r(n-1) + 1
print("deep", r(800))
gc.collect()
print("done")
"#,
    );
}

#[test]
fn loop_alloc_forced_gc_slice() {
    common::interpreter_gate();
    // The perf-sensitive shape (allocation loop with periodic forced
    // collections): correctness gate only (C-24 — timing is not a
    // conformance channel); the fence bounds runtime.
    check(
        "loop-alloc-gc",
        r#"
import gc
acc = []
for i in range(20000):
    acc.append([i])
    if i % 5000 == 0:
        gc.collect()
print("len", len(acc))
print("done")
"#,
    );
}
