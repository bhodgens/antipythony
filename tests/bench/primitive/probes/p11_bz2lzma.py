"""P11 (pp2-bz2): bz2/lzma compress+decompress throughput. JSON lines.

Informational rows (leaf pp2-bz2): decompress is the perf-relevant leg.
bzip2 backend = libbz2-rs-sys (pure Rust, tracks libbz2 closely); lzma
backend = lzma-rs (pure-Rust reference encoder, literals-only — compress
is far off liblzma and NOT a target; decompress is respectable).
Run under CPython 3.14 and antipythony; the driver compares.
"""
import bz2
import json
import lzma
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


def main():
    for n in SIZES:
        data = buf(n)
        cb1 = bz2.compress(data, 1)
        cx = lzma.compress(data)
        emit("bz2.compress(level=1)", n, lambda: bz2.compress(data, 1))
        emit("bz2.decompress(level=1)", n, lambda: bz2.decompress(cb1))
        emit("lzma.compress(xz default)", n, lambda: lzma.compress(data))
        emit("lzma.decompress(xz default)", n, lambda: lzma.decompress(cx))
        ca = lzma.compress(data, format=lzma.FORMAT_ALONE)
        emit("lzma.decompress(alone)", n,
             lambda: lzma.decompress(ca, format=lzma.FORMAT_ALONE))


if __name__ == "__main__":
    main()
