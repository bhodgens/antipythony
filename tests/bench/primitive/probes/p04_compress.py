"""P04: zlib compress/decompress throughput (level 1 and 6). JSON lines.

Note: the mandated buffer (bytes(range(256)) * k) is highly compressible,
so compression throughput here reflects that workload only.
"""
import json
import time
import zlib

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


def main():
    for n in SIZES:
        data = buf(n)
        c1 = zlib.compress(data, 1)
        c6 = zlib.compress(data, 6)
        emit("zlib.compress(level=1)", n, lambda: zlib.compress(data, 1))
        emit("zlib.compress(level=6)", n, lambda: zlib.compress(data, 6))
        emit("zlib.decompress(level=1)", n, lambda: zlib.decompress(c1))
        emit("zlib.decompress(level=6)", n, lambda: zlib.decompress(c6))


if __name__ == "__main__":
    main()
