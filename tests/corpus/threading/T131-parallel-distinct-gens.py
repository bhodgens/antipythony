# T-10 threading corpus T131
# Cites: S-02-20/21 (generator protocol), S-05-4 (Thread), C-24
# Pins: N threads EACH pull their OWN generator concurrently - a generator's
# frame state (locals, cursor) is private to that generator; no
# cross-generator interference under parallel pulls. Order-independent:
# results keyed by worker index and printed sorted.
# Determinism: no sleep; Barrier rendezvous; sorted collection before print.
import threading

N = 8
barrier = threading.Barrier(N + 1)
results = []
results_lock = threading.Lock()

def gen(k):
    for i in range(k + 1):
        yield i * 31 + k

def worker(idx):
    barrier.wait()
    total = 0
    for v in gen(idx):
        total = total + v
    with results_lock:
        results.append((idx, total))

ts = [threading.Thread(target=worker, args=(i,)) for i in range(N)]
for t in ts:
    t.start()
barrier.wait()
for t in ts:
    t.join()
for i, v in sorted(results):
    print(i, v)
