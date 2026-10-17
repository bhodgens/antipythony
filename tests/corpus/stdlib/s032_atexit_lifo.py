# Cites: S-05-4 (row atexit), C-28/C-29, spec 05 atexit-anchor, S-06-11
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import atexit
import sys
print("main-start")
def a():
    print("A-last-registered-runs-first")
def b():
    print("B-first-registered-runs-second")
atexit.register(b)
atexit.register(a)
def un():
    print("UN-should-not-run")
atexit.register(un)
atexit.unregister(un)
print("unregistered", un.__name__)
atexit.register(print, "bye")
sys.exit(3)
