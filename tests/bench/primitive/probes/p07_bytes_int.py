"""P07: int <-> bytes / big decimal str conversions. JSON lines.

Input size = the byte/char length of the big-int operand being converted.
"""
import json
import sys
import time

SIZES = [49152, 1048576]

sys.set_int_max_str_digits(SIZES[-1] * 4)


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


def make_decstr(n):
    # deterministic decimal digit string of length n (leading digit nonzero)
    tail = "1234567890" * (n // 10 + 1)
    return ("9" + tail)[:n]


def main():
    for n in SIZES:
        data = buf(n)
        dec = make_decstr(n)
        big = int.from_bytes(data, "big")
        emit("int.from_bytes", n, lambda: int.from_bytes(data, "big"))
        emit("int.to_bytes", n, lambda: big.to_bytes(n, "big"))
        emit("int(decimal_str)", n, lambda: int(dec))
        emit("hex(big_int)", n, lambda: hex(big))


if __name__ == "__main__":
    main()
