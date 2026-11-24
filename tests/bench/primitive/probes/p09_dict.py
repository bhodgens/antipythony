"""P09: build 1M-entry dict + 1M lookups. JSON lines.

SIZES are element counts (48KiB/1MiB scaled at 8-byte keys) plus the
spec-mandated 1M-entry fixed workload.
"""
import json
import time

KEYS_1M = [i * 2654435761 % (1 << 62) for i in range(1_000_000)]


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


def build(keys):
    return {k: k for k in keys}


def lookups(d, keys):
    hit = 0
    for k in keys:
        if d.get(k) is not None:
            hit += 1
    return hit


def main():
    for count in (6144, 131072):
        keys = KEYS_1M[:count]
        nbytes = count * 8
        d = build(keys)
        emit("dict.build", nbytes, lambda: build(keys))
        emit("dict.lookups", nbytes, lambda: lookups(d, keys))
    d = build(KEYS_1M)
    emit("dict.build_1M", 8_000_000, lambda: build(KEYS_1M))
    emit("dict.lookups_1M", 8_000_000, lambda: lookups(d, KEYS_1M))


if __name__ == "__main__":
    main()
