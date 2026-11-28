"""P06: str find/replace/split/join/upper throughput. JSON lines.

Large-scale str workloads: same deterministic text both interpreters.

Fast-path note: each timed workload is a NAMED MODULE-LEVEL function holding
the measured operation; lambda bodies bypass call_function/tier dispatch.
Module globals carry the per-size inputs so the SAME source runs under
CPython and antipythony with identical semantics. emit()'s protocol and all
row labels are unchanged.
"""
import json
import time

SIZES = [49152, 1048576]


def buf(n):
    return bytes(range(256)) * (n // 256)


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


def make_text(n):
    # printable-only latin-1 text so bytes->str keeps a 1:1 char count
    data = bytes(range(32, 127)) * (n // 95 + 1)
    return data[:n].decode("latin-1")


# Per-size inputs; set by main() before each size's emits.
_TEXT = None
_WORDS = None


def str_find():
    return _TEXT.find("xyzzy")


def str_replace():
    return _TEXT.replace("0", "O")


def str_split():
    return _TEXT.split()


def str_join():
    return "\x00".join(_WORDS)


def str_upper():
    return _TEXT.upper()


def main():
    global _TEXT, _WORDS
    for n in SIZES:
        _TEXT = make_text(n)
        _WORDS = _TEXT.split()
        emit("str.find", n, str_find)
        emit("str.replace", n, str_replace)
        emit("str.split", n, str_split)
        emit("str.join", n, str_join)
        emit("str.upper", n, str_upper)


if __name__ == "__main__":
    main()
