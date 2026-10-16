# T-10 threading corpus T104
# Cites: S-05-4 (threading row: Event, Thread), C-01
# Pins: Event set/wait handshake ordering — workers announce readiness,
# main releases them, completion names printed sorted.
# Determinism: pure Event rendezvous, no timing.
import threading

N = 4
ready = [threading.Event() for _ in range(N)]
go = threading.Event()
done = []
done_lk = threading.Lock()

def worker(i):
    ready[i].set()
    go.wait()
    with done_lk:
        done.append("done-%d" % i)

ts = [threading.Thread(target=worker, args=(i,), name="E%d" % i) for i in range(N)]
for t in ts: t.start()
for r in ready: r.wait()
print("all_ready:", all(r.is_set() for r in ready))
go.set()
for t in ts: t.join()
print("completed:", sorted(done))
print("go still set:", go.is_set())
go.clear()
print("cleared:", go.is_set())
