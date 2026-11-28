# Cites: S-02-33 S-07-28
# Pins: finally runs on BOTH the exception path (cleanup runs before the
# exception leaves the try) and the return path (finally before the value
# returns) — bughunt D11. The escaping exception is caught outside and
# printed (the traceback stderr surface is not pinned on the wasm slice).
def g():
    try:
        return 1
    finally:
        print("fin")
print(g())
try:
    try:
        raise ValueError("boom")
    finally:
        print("cleanup")
except ValueError as e:
    print("caught:", e)
