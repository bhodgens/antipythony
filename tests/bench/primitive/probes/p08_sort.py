"""P08: sorted() over 1M seeded random ints + list.sort. JSON lines.

SIZES here are element counts (not bytes): 96KiB and 2Mi of int objects
are meaningless for a sort; the spec's 1M-element sort is fixed, so the
small size scales the element count and "size" reports element bytes.

Fast-path note: each timed workload is a NAMED MODULE-LEVEL function whose
body contains the hot operation. call_function routes named def bodies
through bytecode/tier dispatch; lambda bodies bypass it via eval_expr.
The emit() warmup / best-of-5 protocol and all row labels are unchanged.
"""
import json
import random
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


def make_data(count, seed):
    rng = random.Random(seed)
    return [rng.randrange(0, 1 << 62) for _ in range(count)]


# Named module-level workload functions (the bodies call_function dispatches).
_SORT_DATA = None
_SORT_COUNT = 0
_SORT_SEED = 0xC0FFEE


def sorted_small():
    return sorted(_SORT_DATA)


def sort_small():
    return make_data(_SORT_COUNT, _SORT_SEED).sort()


def sorted_big():
    return sorted(_SORT_DATA)


def sort_big():
    return make_data(1_000_000, 0xC0FFEE).sort()


def main():
    global _SORT_DATA, _SORT_COUNT
    # element counts scaled from 48KiB / 1MiB (8 bytes per int payload)
    for count in (6144, 131072):
        nbytes = count * 8
        data = make_data(count, 0xC0FFEE)
        _SORT_DATA = data
        _SORT_COUNT = count
        emit("sorted(list[int])", nbytes, sorted_small)
        emit("list.sort", nbytes, sort_small)
    # spec-mandated 1M-element sort, reported once at its byte size
    big = make_data(1_000_000, 0xC0FFEE)
    _SORT_DATA = big
    emit("sorted(list[int])_1M", 8_000_000, sorted_big)
    emit("list.sort_1M", 8_000_000, sort_big)


if __name__ == "__main__":
    main()
