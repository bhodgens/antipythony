# T-10 threading corpus T101
# Cites: S-05-4 (threading row: Thread), C-01, C-24
# Pins: N worker threads started via a rendezvous barrier, joined, and the
# aggregate printed SORTED so the output is completion-order independent.
# Determinism: no sleep; Barrier release + join; sorted collection before print.
import threading

N = 8
start = threading.Barrier(N + 1)
results = []
results_lock = threading.Lock()

def worker(i):
    start.wait()  # all threads leave together (logical order pinned)
    v = (i * 7919) % 1000
    with results_lock:
        results.append((i, v))

ts = [threading.Thread(target=worker, args=(i,), name="W%d" % i) for i in range(N)]
for t in ts:
    t.start()
start.wait()
for t in ts:
    t.join()
print("n_results:", len(results))
for i, v in sorted(results):
    print(i, v)
print("all_joined:", not any(t.is_alive() for t in ts))
