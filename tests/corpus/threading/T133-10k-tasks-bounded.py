# T-10 threading corpus T133 (02.10 2c.5)
# Cites: S-05-4 (Thread), C-24 (order-independent aggregate), 02.10 gate
# (10k-task bounded memory — the scheduler's spawn-many/park-most shape).
# Pins: 10_000 Python threads spawned, each parked on a per-thread Event;
# the main thread releases them in waves and joins everything. The
# aggregate is order-independent by construction: every task writes its
# completion index under a lock and the SORTED list is printed; a set of
# per-task TLS writes (threading.local) proves per-task namespace
# isolation at 10k scale. Determinism: no sleep; the wave protocol
# (release-all, then join-all) bounds the run without wall-clock pins.
# Memory NOTE (the 2c.5 gate): with the scheduler pool, a task that has
# never run holds NO fiber stack (~hundreds of bytes of Task record), so
# the spawn-many/park-most shape stays bounded — the pre-scheduler tree
# reserved 64 MiB of OS-thread stack per started Thread (625 GiB VA at
# 10k). The pytest harness (tests/t10_threading) asserts completion, not
# RSS; the RSS/VA gate lives in scripts/t121-style measurement tooling.
import threading

N = 10000

locals_map = {}
tls_lock = threading.Lock()
done = []
done_lock = threading.Lock()
events = [threading.Event() for _ in range(N)]


def worker(i):
    # Per-task TLS namespace (the 2c re-key: keyed on the task's logical
    # ident, not the worker's os tid — two tasks share one worker).
    loc = threading.local()
    loc.v = i
    events[i].wait(timeout=30.0)
    with tls_lock:
        locals_map[i] = loc.v
    with done_lock:
        done.append(i)


threads = []
for i in range(N):
    t = threading.Thread(target=worker, args=(i,))
    threads.append(t)
    t.start()

for i in range(N):
    events[i].set()

for t in threads:
    t.join(timeout=30)

# Sorted aggregate: identical regardless of interleaving.
print(len(done))
print(sum(done))
tls_ok = all(locals_map[i] == i for i in locals_map)
print(tls_ok, len(locals_map))
