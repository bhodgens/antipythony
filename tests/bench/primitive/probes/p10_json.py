"""P10: json.dumps / loads over a nested ~1MiB structure. JSON lines.

Input size = the serialized JSON byte length; deterministic structure.
"""
import json
import time

SIZES = [49152, 1048576]


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


def make_struct(target_bytes):
    # deterministic nested structure; grows until serialized size >= target
    obj = {"records": [], "meta": {"kind": "bench", "v": 1}}
    i = 0
    while True:
        obj["records"].append({
            "id": i,
            "name": "item-%06d" % i,
            "tags": ["alpha", "beta", "gamma"],
            "score": i * 7919 % 100000,
            "blob": "x" * 40,
        })
        i += 1
        if i % 256 == 0 and len(json.dumps(obj)) >= target_bytes:
            break
    return obj


def main():
    for n in SIZES:
        obj = make_struct(n)
        payload = json.dumps(obj)
        nbytes = len(payload.encode("utf-8"))
        emit("json.dumps", nbytes, lambda: json.dumps(obj))
        emit("json.loads", nbytes, lambda: json.loads(payload))


if __name__ == "__main__":
    main()
