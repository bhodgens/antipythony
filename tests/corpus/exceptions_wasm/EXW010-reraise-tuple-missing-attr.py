# Cites: S-02-30 S-07-28
# Pins: bare `raise` in a handler re-raises the caught exception (D5) — the
# outer handler observes the ORIGINAL payload (print rides stdout; the
# traceback stderr surface is not pinned on the wasm slice, family-wide).
try:
    try:
        raise ValueError("v")
    except ValueError:
        raise
except ValueError as e:
    print("reraised:", e)
