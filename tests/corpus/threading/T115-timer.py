# T-10 threading corpus T115
# Cites: S-05-4 (threading row: Timer), C-01
# Pins: Timer is a Thread subclass; a zero-delay Timer runs its fn and can be
# joined; cancel() before start() prevents any run (join of never-started timer
# raises the exact documented RuntimeError text).
# Determinism: delay 0 + join; the cancelled timer is never started, so its
# body is unreachable by construction.
import threading

hits = []
def ring(tag):
    hits.append(tag)

t = threading.Thread  # sanity: Timer derives from Thread
print("Timer is Thread subclass:", issubclass(threading.Timer, threading.Thread))

tm = threading.Timer(0, ring, args=("zero",))
tm.start()
tm.join()
print("zero-delay fired:", hits == ["zero"], "alive:", tm.is_alive())

tc = threading.Timer(0, ring, args=("cancelled",))
tc.cancel()
print("cancelled not started alive:", tc.is_alive())
try:
    tc.join()
except RuntimeError as e:
    print("join unstarted:", type(e).__name__ + ": " + str(e))
print("final hits:", hits)
