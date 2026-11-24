"""P05: hashlib sha256 / md5 throughput. Prints JSON lines."""
import hashlib
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


def main():
    try:
        hashlib.md5(b"")
    except ValueError:
        print("# note: hashlib.md5 unavailable on this build; md5 rows skipped")
        md5_ok = False
    else:
        md5_ok = True
    for n in SIZES:
        data = buf(n)
        emit("hashlib.sha256", n, lambda: hashlib.sha256(data).digest())
        if md5_ok:
            emit("hashlib.md5", n, lambda: hashlib.md5(data).digest())


if __name__ == "__main__":
    main()
