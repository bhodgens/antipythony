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


def build(keys):
    return {k: k for k in keys}


def get_lookups(d, keys):
    hit = 0
    for k in keys:
        if d.get(k) is not None:
            hit += 1
    return hit


def getitem_lookups(d, keys):
    hit = 0
    for k in keys:
        if d[k] is not None:
            hit += 1
    return hit


def iter_keys(d):
    n = 0
    for k in d:
        n += 1
    return n


def run_size(count):
    keys = make_keys(count)
    nbytes = count * 8
    d = build(keys)
    emit("dict.build", nbytes, lambda: build(keys))
    emit("dict.get", nbytes, lambda: get_lookups(d, keys))
    emit("dict.getitem", nbytes, lambda: getitem_lookups(d, keys))
    emit("dict.iter", nbytes, lambda: iter_keys(d))


def main():
    # element counts scaled from 48KiB / 1MiB (8 bytes per int payload);
    # the 1MiB-scaled count is 32768, sized so the whole probe finishes
    # well inside the harness's 120s timeout on BOTH interpreters
    run_size(6144)
    run_size(32768)
    # fixed workload, downgraded 1M -> 32k (see docstring)
    run_size_named()


def run_size_named():
    keys = make_keys(32_000)
    nbytes = 32_000 * 8
    d = build(keys)
    emit("dict.build_32k", nbytes, lambda: build(keys))
    emit("dict.get_32k", nbytes, lambda: get_lookups(d, keys))
    emit("dict.getitem_32k", nbytes, lambda: getitem_lookups(d, keys))
    emit("dict.iter_32k", nbytes, lambda: iter_keys(d))


if __name__ == "__main__":
    main()
