# T-10 threading corpus T107
# Cites: S-05-4 (threading row: Thread.daemon), C-01, C-28
# Pins: a daemon thread blocked on an Event that is NEVER set cannot keep the
# process alive; the main thread finishes and the process exits 0. The daemon's
# pre-block print is ordered against main via an Event handshake (no race).
# Daemon teardown timing is NOT asserted — the negative (the post-wait print
# never appears) is unreachable by construction.
import threading

gate = threading.Event()
started = threading.Event()

def daemon_body():
    print("daemon-started")
    started.set()
    gate.wait()          # never set -> thread parked forever
    print("daemon-NEVER-runs-this")

d = threading.Thread(target=daemon_body, name="DaemonBlocked", daemon=True)
print("daemon flag before start:", d.daemon)
d.start()
started.wait()
print("daemon flag after start:", d.daemon)
print("main: continuing to exit")
