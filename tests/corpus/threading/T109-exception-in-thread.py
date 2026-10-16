# T-10 threading corpus T109
# Cites: S-05-4 (threading row: excepthook default behavior), S-02-35, S-02-37,
# C-01, C-24, C-25, C-28
# Pins: an uncaught exception in a worker thread is reported to STDERR in the
# CPython 3.14 format — "Exception in thread <name>:" header, the threading.py
# internal frames (_bootstrap_inner / run, with 3.11+ fine-grained markers),
# then the user frame and summary line; process exit code stays 0 and stdout
# still gets the post-join print.
# Determinism: the report is emitted synchronously by the dying thread before
# join returns, so stderr ordering is pinned by the thread-death/join protocol
# (NOT by wall-clock timing).
import threading

def boom():
    raise ValueError("kaboom")

t = threading.Thread(target=boom)
t.start()
t.join()
print("main continues after thread death")
