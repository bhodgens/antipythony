# T-10 threading corpus T132
# Cites: S-02-20/21 (generator protocol, generator expressions), S-05-4
# (Thread), C-24
# Pins: (a) a generator expression consumed across TWO threads yields the
# SAME element SET as single-threaded consumption - concurrent next() on one
# genexp never duplicates or drops an element (exactly one thread sees each
# element; a mid-resume loser raises ValueError). (b) A thread driving its
# own genexp alongside is unaffected (per-generator state isolation).
# Determinism: no sleep; Barrier rendezvous; sorted aggregate before print.
import threading

data = list(range(200))
gx = (x * x for x in data)

draws = []
draw_lock = threading.Lock()
barrier = threading.Barrier(3)

def consumer():
    barrier.wait()
    local = []
    while True:
        try:
            local.append(next(gx))
        except ValueError:
            pass
        except StopIteration:
            break
    with draw_lock:
        draws.extend(local)

def bystander():
    barrier.wait()
    own = (y + 1 for y in range(50))
    s = 0
    for v in own:
        s = s + v
    with draw_lock:
        draws.append(("bystander", s))

ts = [threading.Thread(target=consumer) for _ in range(2)] + [
    threading.Thread(target=bystander)]
for t in ts:
    t.start()
for t in ts:
    t.join()

squares = [x * x for x in data]
elems = sorted(v for v in draws if isinstance(v, int))
other = [v for v in draws if not isinstance(v, int)]
print("distinct_elems:", len(elems), "sum_ok:", elems == sorted(squares))
print("bystander:", other)
