//! Differential name-resolution tests (leaf 02.02 fix pass): each probe runs
//! under CPython 3.14 AND under the ap-run tree-walking executor; stdout
//! bytes and exit code must match byte-exactly (S-06-1/S-06-2).
//!
//! Covers the three fresh-smoke bugs:
//! - BUG-A: module-level function names read from nested frames
//!   (self-recursion + mutual recursion).
//! - BUG-B: escaping closures must keep shared cells (curried factory).
//! - BUG-C: `yield from` delegation (list / str / generator sub-iterables).
//!
//! Plus the nine original gate probes, re-asserted so the fix does not
//! regress them.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic probe-file uniquifier (concurrent tests share one process id,
/// and wall-clock nanos can collide when tests start simultaneously).
static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

// Reference discovery is shared (leaf 00.02): env AP_CONFORM_REFERENCE_PYTHON
// -> PATH python3.14 (version-string checked, never a bare python3), and
// AP_REQUIRE_REFERENCE=1 converts a missing reference into a hard failure.
mod common;

/// Run a probe under the reference interpreter; returns (stdout, exit).
fn run_reference(py: &str, src: &str) -> (Vec<u8>, i32) {
    let dir = std::env::temp_dir().join(format!(
        "aprun-diff-{}-{}",
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

/// Run a probe under ap-run; returns (stdout, exit). Subprocess-per-probe
/// (OOM mitigation): the interpreter runs in a fresh ap-run-probe process,
/// so the tree walk's big Rust frames and the GC heap die with the probe —
/// no dedicated in-process thread stack needed anymore.
fn run_aprun(src: &str) -> (Vec<u8>, i32) {
    let (out, _err, code) = common::run_aprun_eval(src);
    (out, code)
}

fn assert_differential(name: &str, src: &str) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let (ref_out, ref_exit) = run_reference(py, src);
    let (ap_out, ap_exit) = run_aprun(src);
    assert_eq!(
        String::from_utf8_lossy(&ap_out),
        String::from_utf8_lossy(&ref_out),
        "{name}: stdout diverges from CPython 3.14\n--- source ---\n{src}"
    );
    assert_eq!(
        ap_exit, ref_exit,
        "{name}: exit code diverges\n--- source ---\n{src}"
    );
}

// ===========================================================================
// BUG-A: module-level fn names read from inside function frames
// ===========================================================================

#[test]
fn bug_a_self_recursion_fact() {
    common::interpreter_gate();
    assert_differential(
        "self-recursion fact(5)",
        "def fact(n):\n    return 1 if n <= 1 else n * fact(n-1)\nprint(fact(5))\n",
    );
}

#[test]
fn bug_a_recursion_depth_3() {
    common::interpreter_gate();
    assert_differential(
        "recursion depth 3",
        "def d3(k):\n    return 0 if k == 0 else 1 + d3(k-1)\nprint(d3(3))\nprint(d3(7))\n",
    );
}

#[test]
fn bug_a_mutual_recursion_three_fns() {
    common::interpreter_gate();
    assert_differential(
        "mutual 3 fns (forward reference)",
        concat!(
            "def is_even(n):\n",
            "    return True if n == 0 else is_odd(n - 1)\n",
            "def is_odd(n):\n",
            "    return False if n == 0 else is_even(n - 1)\n",
            "def ping(n):\n",
            "    return 0 if n == 0 else 1 + pong(n - 1)\n",
            "def pong(n):\n",
            "    return 0 if n == 0 else 2 + ping(n - 1)\n",
            "print(is_even(10))\n",
            "print(is_odd(7))\n",
            "print(ping(5))\n",
            "print(pong(4))\n",
        ),
    );
}

#[test]
fn bug_a_def_after_use_in_call_order() {
    common::interpreter_gate();
    // The def executes at module time before any call; a fn defined LATER
    // in the module must still be visible to an earlier-defined fn.
    assert_differential(
        "later def visible to earlier def",
        concat!(
            "def early():\n    return late() + 1\n",
            "def late():\n    return 41\n",
            "print(early())\n",
        ),
    );
}

#[test]
fn bug_a_global_value_read_still_works() {
    common::interpreter_gate();
    assert_differential(
        "global value read from fn",
        "G = 42\ndef f():\n    return G\nprint(f())\n",
    );
}

#[test]
fn bug_a_module_fn_reassigned_read_from_fn() {
    common::interpreter_gate();
    // Name resolution must see the CURRENT global binding at call time.
    assert_differential(
        "module fn rebinding",
        concat!(
            "def a():\n    return 1\n",
            "def b():\n    return 2\n",
            "def caller(f):\n    return f()\n",
            "print(caller(a))\n",
            "a, b = b, a\n",
            "print(caller(a))\n",
        ),
    );
}

// ===========================================================================
// BUG-B: escaping closures keep their cells
// ===========================================================================

#[test]
fn bug_b_curried_closure_escapes() {
    common::interpreter_gate();
    assert_differential(
        "curried closure",
        "def mk(n):\n    def inner(x):\n        return x + n\n    return inner\nprint(mk(10)(5))\n",
    );
}

#[test]
fn bug_b_closure_factory_two_closures_share_cell() {
    common::interpreter_gate();
    assert_differential(
        "two closures share one cell",
        concat!(
            "def mk():\n",
            "    count = 0\n",
            "    def inc():\n",
            "        nonlocal count\n",
            "        count += 1\n",
            "        return count\n",
            "    def get():\n",
            "        return count\n",
            "    return inc, get\n",
            "a, b = mk()\n",
            "print(a())\n",
            "print(a())\n",
            "print(b())\n",
            "b2 = mk()\n",
            "print(b2[1]())\n",
        ),
    );
}

#[test]
fn bug_b_closure_direct_call_still_works() {
    common::interpreter_gate();
    assert_differential(
        "closure called inside maker",
        "def mk():\n    x = 5\n    def inner():\n        return x\n    return inner()\nprint(mk())\n",
    );
}

#[test]
fn bug_b_param_cell_two_levels() {
    common::interpreter_gate();
    // The closed-over variable is a PARAMETER of the maker, captured two
    // levels deep (the exact mk(10)(5) shape, plus rebinding through a cell).
    assert_differential(
        "param cell, two-level capture",
        concat!(
            "def outer(base):\n",
            "    def mid():\n",
            "        def inner():\n",
            "            return base * 2\n",
            "        return inner()\n",
            "    return mid\n",
            "print(outer(21)())\n",
        ),
    );
}

#[test]
fn bug_b_factory_loop_closure_captures_cell() {
    common::interpreter_gate();
    // Classic late-binding: all closures share ONE cell per iteration scope.
    assert_differential(
        "factory loop shares cell",
        concat!(
            "def mk():\n",
            "    fns = []\n",
            "    for i in [1, 2, 3]:\n",
            "        fns.append(lambda: i)\n",
            "    return fns\n",
            "fs = mk()\n",
            "print(fs[0](), fs[1](), fs[2]())\n",
        ),
    );
}

// ===========================================================================
// BUG-C: yield from delegation
// ===========================================================================

#[test]
fn bug_c_yield_from_range() {
    common::interpreter_gate();
    assert_differential(
        "yield from range",
        concat!(
            "def g(n):\n",
            "    yield from range(n)\n",
            "print(list(g(4)))\n",
        ),
    );
}

#[test]
fn bug_c_yield_from_list_and_str() {
    common::interpreter_gate();
    assert_differential(
        "yield from list + str",
        concat!(
            "def g():\n",
            "    yield from [10, 20]\n",
            "    yield from 'ab'\n",
            "    yield 99\n",
            "print(list(g()))\n",
        ),
    );
}

#[test]
fn bug_c_yield_from_generator() {
    common::interpreter_gate();
    // Sub-iterable is itself a generator (delegation through delegation).
    assert_differential(
        "yield from generator",
        concat!(
            "def inner(n):\n",
            "    for i in range(n):\n",
            "        yield i * i\n",
            "def outer(n):\n",
            "    yield from inner(n)\n",
            "    yield from inner(2)\n",
            "print(list(outer(3)))\n",
        ),
    );
}

#[test]
fn bug_c_yield_from_mixed_with_plain_yields() {
    common::interpreter_gate();
    assert_differential(
        "yield from mixed with plain yields",
        concat!(
            "def g():\n",
            "    yield 0\n",
            "    yield from [1, 2]\n",
            "    yield 3\n",
            "    yield from (x for x in [4, 5])\n",
            "print(list(g()))\n",
        ),
    );
}

// ===========================================================================
// The nine original gate probes (regression guard for the fix pass)
// ===========================================================================

#[test]
fn gate_gen_list() {
    common::interpreter_gate();
    assert_differential(
        "gen list",
        concat!(
            "def g():\n",
            "    yield 1\n",
            "    yield 2\n",
            "print(list(g()))\n",
            "print(sum(g()))\n",
        ),
    );
}

#[test]
fn gate_gen_comp_via_yield_from() {
    common::interpreter_gate();
    assert_differential(
        "gen comp via yield from",
        concat!(
            "def g(n):\n",
            "    yield from (x * 2 for x in range(n))\n",
            "print(list(g(4)))\n",
        ),
    );
}

#[test]
fn gate_gen_next() {
    common::interpreter_gate();
    assert_differential(
        "gen next",
        concat!(
            "def g():\n",
            "    yield 7\n",
            "    yield 8\n",
            "it = g()\n",
            "print(next(it))\n",
            "print(next(it))\n",
            "try:\n",
            "    next(it)\n",
            "except StopIteration:\n",
            "    print('stop')\n",
        ),
    );
}

#[test]
fn gate_closure_direct() {
    common::interpreter_gate();
    assert_differential(
        "closure direct",
        concat!(
            "def counter():\n",
            "    n = 0\n",
            "    def bump():\n",
            "        nonlocal n\n",
            "        n += 1\n",
            "        return n\n",
            "    return bump\n",
            "c = counter()\n",
            "print(c(), c(), c())\n",
        ),
    );
}

#[test]
fn gate_closure_curried() {
    common::interpreter_gate();
    assert_differential(
        "closure curried",
        concat!(
            "def add(a):\n",
            "    return lambda b: a + b\n",
            "print(add(2)(3))\n",
            "plus5 = add(5)\n",
            "print(plus5(10))\n",
        ),
    );
}

#[test]
fn gate_unpack() {
    common::interpreter_gate();
    assert_differential(
        "unpack",
        concat!(
            "a, b = 1, 2\n",
            "print(a, b)\n",
            "a, b = b, a\n",
            "print(a, b)\n",
            "x, *rest = [1, 2, 3, 4]\n",
            "print(x, rest)\n",
            "*init, last = [9, 8, 7]\n",
            "print(init, last)\n",
        ),
    );
}

#[test]
fn gate_match() {
    common::interpreter_gate();
    assert_differential(
        "match",
        concat!(
            "def classify(v):\n",
            "    match v:\n",
            "        case 0:\n",
            "            return 'zero'\n",
            "        case [x, y]:\n",
            "            return f'pair {x} {y}'\n",
            "        case {'k': val}:\n",
            "            return f'map {val}'\n",
            "        case _:\n",
            "            return 'other'\n",
            "print(classify(0))\n",
            "print(classify([1, 2]))\n",
            "print(classify({'k': 5}))\n",
            "print(classify(3.5))\n",
        ),
    );
}

#[test]
fn gate_try_finally() {
    common::interpreter_gate();
    assert_differential(
        "try/finally",
        concat!(
            "def f():\n",
            "    out = []\n",
            "    try:\n",
            "        out.append('try')\n",
            "        raise ValueError('boom')\n",
            "    except ValueError as e:\n",
            "        out.append(f'caught {e}')\n",
            "    finally:\n",
            "        out.append('finally')\n",
            "    return out\n",
            "print(f())\n",
            "log = []\n",
            "try:\n",
            "    log.append('t')\n",
            "finally:\n",
            "    log.append('f')\n",
            "print(log)\n",
        ),
    );
}

#[test]
fn gate_recursion() {
    common::interpreter_gate();
    // Depth kept below ~90 per chain: the tier-0 tree-walk recurses through
    // large Rust stack frames (deep recursion overflow-aborts at ~93 chained
    // calls in a debug build with the default 8MB main-thread stack) — the
    // RecursionError-at-1000 limit is unreachable in debug binaries. Not a
    // regression of this fix pass (identical pre-fix); noted for 02.05.
    assert_differential(
        "recursion",
        concat!(
            "def fib(n):\n",
            "    return n if n < 2 else fib(n-1) + fib(n-2)\n",
            "print(fib(10))\n",
            "def sum_to(n):\n",
            "    return 0 if n == 0 else n + sum_to(n - 1)\n",
            "print(sum_to(40))\n",
        ),
    );
}
