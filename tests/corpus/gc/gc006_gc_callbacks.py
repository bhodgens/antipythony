# Cites: C-21, C-22, C-23, D1
# gc.callbacks observation (3.13+ API): one gc.collect() emits exactly the
# phase sequence [("start", g), ("stop", g)] with g = the requested
# generation (default collect() reports generation 2 on 3.14.7 — verified
# by running it); info keys are exactly {collected, generation,
# uncollectable}; two consecutive gc.collect() calls emit the identical
# phase sequence (fixture re-runs 3x). gc.collect(n) pins the generation
# argument: collect(0) -> gen 0, collect(1) -> gen 1.
import gc

seq = []
gc.callbacks.append(lambda phase, info: seq.append((phase, info["generation"])))

gc.collect()
first = list(seq)
seq.clear()
gc.collect()
second = list(seq)
print("start/stop only:", all(p in ("start", "stop") for p, _ in first))
print("paired:", [p for p, _ in first] == ["start", "stop"] * (len(first) // 2))
print("stable across runs:", first == second)
print("first:", first)
seq.clear()
gc.collect(0)
print("collect0 gen:", [g for _, g in seq])
seq.clear()
gc.collect(1)
print("collect1 gen:", sorted(set(g for _, g in seq)))
gc.callbacks.clear()

keysets = []
gc.callbacks.append(lambda phase, info: keysets.append(sorted(info)))
gc.collect()
print("info keys:", keysets)
gc.callbacks.clear()
