# T-10 threading corpus T105
# Cites: S-05-4 (threading row: Lock, RLock), C-01
# Pins: Lock blocking/non-blocking acquire + release bookkeeping; RLock
# reentrancy depth; `with` protocol on both. Cross-thread checks are sequenced
# by start/join so every printed line is a deterministic single-threaded
# observation.
# Determinism: acquire/release counts are pure bookkeeping, no timing.
import threading

lk = threading.Lock()
print("lock initial free:", lk.acquire(blocking=False))
lk.release()
with lk:
    print("lock held inside with:", not lk.acquire(blocking=False))
print("lock free after with:", lk.acquire(blocking=False))
lk.release()

rl = threading.RLock()
owner = {}
def hold_deep():
    with rl:
        with rl:
            with rl:
                owner["depth_ok"] = True
def grab_free():
    # runs AFTER hold_deep joined: the lock is free again (balanced exits)
    owner["other_got"] = rl.acquire(blocking=False)
    rl.release()
t1 = threading.Thread(target=hold_deep, name="D")
t1.start(); t1.join()
t2 = threading.Thread(target=grab_free, name="O")
t2.start(); t2.join()
print("reentrant depth 3:", owner["depth_ok"])
print("free after balanced nested release:", owner["other_got"])
print("lock still balanced:", lk.acquire(blocking=False))
lk.release()
