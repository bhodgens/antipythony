# T-10 threading corpus T130
# Cites: S-02-20/21 (generator protocol), S-05-4 (Thread), C-24
# Pins: TWO threads pulling ONE generator CONCURRENTLY. CPython semantics:
# exactly one pull wins each resume; the loser raises ValueError
# ('generator already executing'). Order-independent by construction: the
# winner's value and the loser's error kind are collected under a lock and
# printed sorted; the aggregate is identical regardless of which thread wins.
# Determinism: no sleep; Barrier rendezvous; the CPU loop inside the body
# only widens the overlap window - no wall-clock dependence.
import threading

def g():
    for i in range(3):
        s = 0
        for j in range(40000000):
            s = (s + j * 3) % 1000000007
        yield s

it = g()
first = next(it)  # start the generator; suspended at first yield
barrier = threading.Barrier(2)
outcomes = []
out_lock = threading.Lock()

def pull():
    barrier.wait()
    try:
        v = next(it)
        with out_lock:
            outcomes.append(("value", v))
    except ValueError:
        with out_lock:
            outcomes.append(("error", "ValueError"))
    except StopIteration:
        with out_lock:
            outcomes.append(("error", "StopIteration"))

ts = [threading.Thread(target=pull) for _ in range(2)]
for t in ts:
    t.start()
for t in ts:
    t.join()
outcomes.sort()
print("first:", first)
for kind, v in outcomes:
    print(kind, v)
