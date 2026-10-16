# T-10 threading corpus T112
# Cites: S-05-4 (threading row: Semaphore, BoundedSemaphore), C-01
# Pins: Semaphore counts permits (blocking=False probes); BoundedSemaphore
# refuses an over-release (release past the initial count) with exact
# ValueError text, raised from a WORKER thread and caught there so the printed
# line is deterministic `Type: message` text assembled by the case.
# Determinism: acquire/release bookkeeping; the violating release runs only
# after a balanced acquire/release, sequenced by start/join.
import threading

s = threading.Semaphore(2)
print("two acquired:", s.acquire(blocking=False), s.acquire(blocking=False))
print("third blocked acquire fails:", not s.acquire(blocking=False))
s.release()
print("after release one free:", s.acquire(blocking=False))

bs = threading.BoundedSemaphore(1)
bs.acquire()
bs.release()  # balanced: release from ANY thread is legal
box = {}
def over_release():
    try:
        bs.release()  # now value == initial: this release is over the bound
        box["r"] = "no error"
    except ValueError as e:
        box["r"] = "ValueError: " + str(e)
t = threading.Thread(target=over_release, name="OR")
t.start(); t.join()
print("bounded over-release:", box["r"])
print("plain semaphore over-release allowed:", end=" ")
ps = threading.Semaphore(1)
ps.release()
print(ps.acquire(blocking=False), ps.acquire(blocking=False))
