# T-10 threading corpus T114
# Cites: C-28 (SystemExit in a non-main thread ends THAT thread, does not set
# the process exit code; only main-thread SystemExit sets it), S-05-4, C-01
# Pins: a worker raising SystemExit(5) simply ends; main thread joins it and
# the process exits 0.
# Determinism: thread-death/join protocol, no timing.
import threading

def exits():
    raise SystemExit(5)

t = threading.Thread(target=exits, name="Exiter")
t.start()
t.join()
print("worker ended:", not t.is_alive())
print("main continues, process exit 0")
