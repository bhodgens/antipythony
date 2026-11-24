"""P01: base64 / binascii encode+decode throughput. Prints JSON lines."""
import base64
import binascii
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
    for n in SIZES:
        data = buf(n)
        enc = base64.b64encode(data)
        b2a = binascii.b2a_base64(data)
        emit("base64.b64encode", n, lambda: base64.b64encode(data))
        emit("base64.b64decode", n, lambda: base64.b64decode(enc))
        emit("binascii.b2a_base64", n, lambda: binascii.b2a_base64(data))
        emit("binascii.a2b_base64", n, lambda: binascii.a2b_base64(b2a))


if __name__ == "__main__":
    main()
