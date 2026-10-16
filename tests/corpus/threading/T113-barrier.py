# T-10 threading corpus T113
# Cites: S-05-4 (threading row: Barrier, BrokenBarrierError), C-01
# Pins: Barrier(4) releases all four parties, then parties/n_waiting
# book-keeping; a timeout on an unmet Barrier raises BrokenBarrierError (exact
# str pinned) and leaves barrier.broken True.
# Determinism: the barrier itself is the rendezvous; the timeout case runs
# alone on the main thread.
import threading

b = threading.Barrier(4)
pres, posts = [], []
lk = threading.Lock()
def party(i):
    with lk:
        pres.append(i)
    b.wait()
    with lk:
        posts.append(i)
ts = [threading.Thread(target=party, args=(i,), name="B%d" % i) for i in range(4)]
for t in ts: t.start()
for t in ts: t.join()
print("all crossed:", sorted(pres), sorted(posts))
print("broken after success:", b.broken)
print("parties:", b.parties, "waiting:", b.n_waiting)

solo = threading.Barrier(2)
try:
    solo.wait(timeout=0.05)
except threading.BrokenBarrierError as e:
    print("timeout:", type(e).__name__ + ": " + repr(str(e)))
print("broken after timeout:", solo.broken)
