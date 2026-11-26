"""P11: crc32/adler32 throughput. JSON lines (leaf pp2-crc)."""
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
        emit("zlib.crc32", n, lambda: zlib.crc32(data))
        emit("zlib.adler32", n, lambda: zlib.adler32(data))


if __name__ == "__main__":
    main()
