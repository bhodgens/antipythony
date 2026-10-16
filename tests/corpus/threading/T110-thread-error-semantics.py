# T-10 threading corpus T110
# Cites: S-05-4 (threading row: Thread, Lock, RLock, ThreadError), C-01
# Pins: exact error text of threading misuse, every check either single-threaded
# or sequenced by join so each printed line is deterministic `Type: message`
# text assembled by the case itself (no traceback gating here).
# Determinism: start/join sequencing; daemon-setter check on an unstarted thread.
import threading

def catch(fn):
    try:
        fn()
        return "no error"
    except Exception as e:
        return type(e).__name__ + ": " + str(e)

lk = threading.Lock()
print("release unlocked lock:", catch(lk.release))

rl = threading.RLock()
box = {}
def foreign_release():
    box["err"] = catch(rl.release)
t = threading.Thread(target=foreign_release, name="FR")
t.start(); t.join()
print("foreign rlock release:", box["err"])

t2 = threading.Thread(target=lambda: None)
print("join before start:", catch(t2.join))
t3 = threading.Thread(target=lambda: None)
t3.start()
print("double start:", catch(t3.start))
t3.join()
t4 = threading.Thread(target=lambda: None, name="Setter")
t4.daemon = True
print("daemon set before start ok:", t4.daemon)
try:
    t3.daemon = True
except RuntimeError as e:
    print("daemon set after start:", "RuntimeError: " + str(e))
print("ThreadError subclass of RuntimeError:", issubclass(threading.ThreadError, RuntimeError))
