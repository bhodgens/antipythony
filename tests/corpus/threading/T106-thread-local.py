# T-10 threading corpus T106
# Cites: S-05-4 (threading row: local), C-01, D1
# Pins: threading.local gives each thread an independent namespace; main thread
# sees none of the worker-written attributes after joins (no refcount/destroy
# ordering is asserted — only what a thread reads its OWN local as).
# Determinism: results collected per-thread, printed sorted by name.
import threading

tl = threading.local()
seen = {}
lk = threading.Lock()

def worker(i):
    tl.value = i * i
    tl.tag = "t%d" % i
    with lk:
        seen[threading.current_thread().name] = (tl.value, tl.tag)

print("main has value:", hasattr(tl, "value"))
ts = [threading.Thread(target=worker, args=(i,), name="TL%d" % i) for i in range(3)]
for t in ts: t.start()
for t in ts: t.join()
for name, (v, tag) in sorted(seen.items()):
    print(name, v, tag)
print("main still lacks value:", hasattr(tl, "value"))
