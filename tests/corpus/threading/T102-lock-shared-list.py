# T-10 threading corpus T102
# Cites: S-05-4 (threading row: Lock), C-01
# Pins: shared list mutated only under a Lock; per-tag counts printed sorted.
# Determinism: lock discipline makes the multiset deterministic; output sorted.
import threading

M, K = 3, 25
shared = []
lk = threading.Lock()

def worker(tag):
    for j in range(K):
        with lk:
            shared.append((tag, j))

ts = [threading.Thread(target=worker, args=(t,)) for t in "abc"]
for t in ts: t.start()
for t in ts: t.join()
print("total:", len(shared))
tags = {}
for tag, _ in shared:
    tags[tag] = tags.get(tag, 0) + 1
print("per-tag:", sorted(tags.items()))
print("each-tag-complete:", all(sorted(j for tg, j in shared if tg == tag) == list(range(K)) for tag in "abc"))
