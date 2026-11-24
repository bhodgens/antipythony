"""P08: sorted() over 1M seeded random ints + list.sort. JSON lines.

SIZES here are element counts (not bytes): 96KiB and 2Mi of int objects
are meaningless for a sort; the spec's 1M-element sort is fixed, so the
small size scales the element count and "size" reports element bytes.
"""
import json
import random
import time


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


def make_data(count, seed):
    rng = random.Random(seed)
    return [rng.randrange(0, 1 << 62) for _ in range(count)]


def main():
    # element counts scaled from 48KiB / 1MiB (8 bytes per int payload)
    for count in (6144, 131072):
        nbytes = count * 8
        data = make_data(count, 0xC0FFEE)
        emit("sorted(list[int])", nbytes, lambda: sorted(data))
        emit("list.sort", nbytes,
             lambda: make_data(count, 0xC0FFEE).sort())
    # spec-mandated 1M-element sort, reported once at its byte size
    big = make_data(1_000_000, 0xC0FFEE)
    emit("sorted(list[int])_1M", 8_000_000, lambda: sorted(big))
    emit("list.sort_1M", 8_000_000, lambda: make_data(1_000_000, 0xC0FFEE).sort())


if __name__ == "__main__":
    main()
