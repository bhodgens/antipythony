# Cites: S-05-4 (row threading, 02.07); spec 06 §6.7 logical ordering only
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import threading
names = ["Thread", "current_thread", "main_thread", "active_count",
         "enumerate", "get_ident", "get_native_id", "Lock", "RLock",
         "Condition", "Event", "Semaphore", "BoundedSemaphore", "Barrier",
         "BrokenBarrierError", "Timer", "local", "TIMEOUT_MAX",
         "ThreadError", "excepthook", "ExceptHookArgs"]
print(len(names), all(hasattr(threading, n) for n in names))
print(threading.current_thread().name == "MainThread",
      threading.current_thread() is threading.main_thread())
print(threading.active_count() >= 1, len(threading.enumerate()) >= 1)
print(type(threading.get_ident()) is int, type(threading.get_native_id()) is int)
results = []
lock = threading.Lock()
def work(i):
    with lock:
        results.append(i * 2)
ts = [threading.Thread(target=work, args=(i,), name=f"w{i}") for i in range(4)]
for t in ts:
    t.start()
for t in ts:
    t.join()
    print(t.name, not t.is_alive())
print(sorted(results))
ev = threading.Event()
ev.set()
print(ev.is_set(), ev.wait(0))
ev.clear()
print(not ev.is_set())
sem = threading.Semaphore(2)
sem.acquire()
sem.acquire()
print(sem.acquire(blocking=False) is False)
sem.release()
print(sem.acquire(blocking=False) is True)
bs = threading.BoundedSemaphore(1)
bs.acquire()
try:
    bs.release()
    bs.release()
except ValueError as e:
    print("bounded:", e)
rl = threading.RLock()
with rl:
    with rl:
        print("rlock-reentrant")
loc = threading.local()
loc.v = 1
print(loc.v)
print(threading.TIMEOUT_MAX > 0,
      issubclass(threading.ThreadError, RuntimeError) or
      issubclass(threading.ThreadError, Exception))
print(callable(threading.excepthook), threading.ExceptHookArgs is not None)
print(threading.BrokenBarrierError is not None, threading.Timer is not None,
      threading.Barrier is not None, threading.Condition is not None)
print(threading.main_thread().name == "MainThread")
