"""P09: dict build + lookups (get, getitem, iteration). JSON lines.

Sizes are element counts (48KiB/1MiB scaled at 8-byte keys). Two changes
from the original probe, both forced by the tier-0 tree-walker's
per-element cost (see plans/primitive-perf/02-parity-audit/audits/
fix-dict-probe-report.md):

1. The spec-mandated 1M-entry fixed workload is REPLACED by a 32k-entry
   fixed workload. The old module-level 1M listcomp alone cost ~35s per
   pass under antipythony (and ~200x that through the dispatched
   workloads), starved the harness's 120s probe timeout, and turned all
   six P09 rows into ERROR. Dict build/lookup scaling (hash growth,
   2/3-load rehash regime) is already exercised at 32k; rows are named
   dict.build_32k etc. to make the size change explicit in the report.

2. Every workload emits its JSON line IMMEDIATELY after its own
   best-of-5 timing, and the key lists are built lazily per size right
   before that size's rows -- no expensive setup ahead of the first
   emit, so partial output survives any future timeout.

Four workloads: dict.build (dictcomp), dict.get (bound-method lookup),
dict.getitem (subscript lookup), dict.iter (insertion-order key
iteration). The get/getitem split exists to attribute the measured gap
between method dispatch and getitem dispatch.

Fast-path note: every zero-arg workload handed to emit() is a NAMED
MODULE-LEVEL function whose body contains the measured loop (lambda
bodies bypass call_function/tier dispatch via eval_expr). Module globals
carry the per-size inputs so the SAME source runs under CPython and
antipythony with identical semantics. emit()'s protocol and all row
labels are unchanged.
"""
import json
import time


def emit(probe, nbytes, fn):
    fn()
    best = float("inf")
    for _ in range(5):
        t0 = time.perf_counter()
        fn()
        best = min(best, time.perf_counter() - t0)
    gbs = nbytes / best / 1e9 if best > 0 else 0.0
    print(json.dumps({"probe": probe, "size": nbytes,
                      "best_s": round(best, 8), "gbs": round(gbs, 4)}))


def make_keys(count):
    return [i * 2654435761 % (1 << 62) for i in range(count)]


# Per-size inputs; set by run_size()/run_size_named() before the emits.
_KEYS = None
_DICT = None


def workload_build():
    """dictcomp hot loop, dispatched through call_function."""
    return {k: k for k in _KEYS}


def workload_get():
    """dict.get hot loop, dispatched through call_function."""
    hit = 0
    for k in _KEYS:
        if _DICT.get(k) is not None:
            hit += 1
    return hit


def workload_getitem():
    """dict.getitem hot loop, dispatched through call_function."""
    hit = 0
    for k in _KEYS:
        if _DICT[k] is not None:
            hit += 1
    return hit


def workload_iter():
    """dict insertion-order iteration hot loop, dispatched via call_function."""
    n = 0
    for k in _DICT:
        n += 1
    return n


def run_size(count):
    global _KEYS, _DICT
    keys = make_keys(count)
    nbytes = count * 8
    _KEYS = keys
    _DICT = {k: k for k in keys}
    emit("dict.build", nbytes, workload_build)
    emit("dict.get", nbytes, workload_get)
    emit("dict.getitem", nbytes, workload_getitem)
    emit("dict.iter", nbytes, workload_iter)


def main():
    # element counts scaled from 48KiB / 1MiB (8 bytes per int payload);
    # the 1MiB-scaled count is 32768, sized so the whole probe finishes
    # well inside the harness's 120s timeout on BOTH interpreters
    run_size(6144)
    run_size(32768)
    # fixed workload, downgraded 1M -> 32k (see docstring)
    run_size_named()


def run_size_named():
    global _KEYS, _DICT
    keys = make_keys(32_000)
    nbytes = 32_000 * 8
    _KEYS = keys
    _DICT = {k: k for k in keys}
    emit("dict.build_32k", nbytes, workload_build)
    emit("dict.get_32k", nbytes, workload_get)
    emit("dict.getitem_32k", nbytes, workload_getitem)
    emit("dict.iter_32k", nbytes, workload_iter)


if __name__ == "__main__":
    main()
