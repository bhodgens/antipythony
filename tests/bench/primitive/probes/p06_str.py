"""P06: str find/replace/split/join/upper throughput. JSON lines.

Large-scale str workloads: same deterministic text both interpreters.
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


def main():
    for n in SIZES:
        text = make_text(n)
        words = text.split()
        emit("str.find", n, lambda: text.find("xyzzy"))
        emit("str.replace", n, lambda: text.replace("0", "O"))
        emit("str.split", n, lambda: text.split())
        emit("str.join", n, lambda: "\x00".join(words))
        emit("str.upper", n, lambda: text.upper())


if __name__ == "__main__":
    main()
