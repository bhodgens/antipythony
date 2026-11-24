"""P03: binascii hexlify / unhexlify throughput. Prints JSON lines."""
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
        hx = binascii.hexlify(data)
        emit("binascii.hexlify", n, lambda: binascii.hexlify(data))
        emit("binascii.unhexlify", n, lambda: binascii.unhexlify(hx))


if __name__ == "__main__":
    main()
