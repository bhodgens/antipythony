"""pp2-sort isolation probe (fixc_dict.py-style): SORT-ONLY cost with the
data build hoisted OUT of the timed region. The P08 harness builds the list
inside the timed lambda for list.sort rows, so its numbers measure the
known interpreter-loop/listcomp floor, not the sort. This probe isolates:
- raw int comparator via list.sort on a prebuilt copy
- raw int comparator via sorted() on the prebuilt list
- presorted best case (n-1 compares: per-element loop floor + comparator)
- str comparator via sorted()
"""
import json
import time


def timeit(fn, reps=5):
    best = None
    for _ in range(reps):
        t0 = time.perf_counter()
        fn()
        dt = time.perf_counter() - t0
        if best is None or dt < best:
            best = dt
    return best


def make_rng(n, seed):
    out = []
    x = seed
    for _ in range(n):
        x = (x * 6364136223846793005 + 1442695040888963407) % (1 << 62)
        out.append(x)
    return out


big = make_rng(1000000, 0xC0FFEE)
big_presorted = sorted(big)
strs100k = ["k" + str(i % 977) for i in range(100000)]
int131k = make_rng(131072, 0xC0FFEE)

out = {}
out["lsort_copy_1M_s"] = timeit(lambda: list(big).sort(), 3)
out["sorted_1M_s"] = timeit(lambda: sorted(big), 3)
out["sorted_1M_presorted_s"] = timeit(lambda: sorted(big_presorted), 3)
out["lsort_copy_131k_s"] = timeit(lambda: list(int131k).sort(), 5)
out["sorted_131k_s"] = timeit(lambda: sorted(int131k), 5)
out["sorted_100k_str_s"] = timeit(lambda: sorted(strs100k), 5)
print(json.dumps(out), flush=True)
