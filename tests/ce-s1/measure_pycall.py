# Leaf 10.04 measurement driver: PyObject_Call bridge cost (looped).
# Same source both runtimes; wall-clock printed to stderr so stdout stays
# byte-comparable. The antipythony run measures C->Python->C round-trip
# cost (extension apply() -> runtime_pycall -> call_value -> lambda).
import callback
import sys
from time import perf_counter

N = 200_000


def inc(x):
    return x + 1


# warm
for _ in range(1000):
    callback.apply(inc, 1)

t0 = perf_counter()
for i in range(N):
    callback.apply(inc, i)
t1 = perf_counter()
per_call_us = (t1 - t0) / N * 1e6
print(f"RESULT per-call-us={per_call_us:.3f}", file=sys.stderr)
sys.stdout.flush()
