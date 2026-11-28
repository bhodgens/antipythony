# Cites: S-02-28 S-07-28
# Pins: bare `raise` OUTSIDE any handler raises RuntimeError with the
# CPython-exact message "No active exception to reraise" (bughunt D5);
# caught and printed here (the traceback stderr surface is not pinned on
# the wasm slice).
def f():
    raise
try:
    f()
except RuntimeError as e:
    print("RuntimeError:", e)
