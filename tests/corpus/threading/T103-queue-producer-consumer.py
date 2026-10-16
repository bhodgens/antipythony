# T-10 threading corpus T103
# Cites: S-05-4 (threading row: Thread; queue use for deterministic rendezvous), C-01
# Pins: queue.Queue producer/consumer with a sentinel; exact multiset received.
# Determinism: sentinel rendezvous + join; consumer drains all, output sorted.
# NOTE: CONTRACT-GAP — `queue` is not listed in spec 05 5.3.2 IN table.
import queue
import threading

SENTINEL = None
items = list(range(200))
q = queue.Queue()

def producer():
    for it in items:
        q.put(it)
    q.put(SENTINEL)

received = []
def consumer():
    while True:
        v = q.get()
        if v is SENTINEL:
            break
        received.append(v)

p = threading.Thread(target=producer, name="P")
c = threading.Thread(target=consumer, name="C")
c.start()
p.start()
p.join()
c.join()
print("count:", len(received))
print("exact multiset:", sorted(received) == items)
print("qsize drained:", q.qsize())
