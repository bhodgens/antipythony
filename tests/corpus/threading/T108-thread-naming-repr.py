# T-10 threading corpus T108
# Cites: S-05-4 (threading row: current_thread, main_thread, enumerate,
# active_count, get_ident), C-01
# Pins: default auto-numbered names (3.14 appends the target function name),
# explicit names, active_count through a rendezvous, repr HEAD only (no idents,
# no " at 0x..." addresses — those are UNSPECIFIED/address-bearing and masked).
# Determinism: names/counts sorted; address-bearing repr fragments masked out.
import threading

def noop():
    pass

anon = [threading.Thread(target=noop) for _ in range(3)]
print("default names:", sorted(t.name for t in anon))
named = [threading.Thread(target=noop, name="H%d" % i) for i in range(3)]
ready = [threading.Event() for _ in range(3)]
gate = threading.Event()
def hold(i):
    ready[i].set()
    gate.wait()
held = [threading.Thread(target=hold, args=(i,), name="K%d" % i) for i in range(3)]
print("before start active:", threading.active_count())
for t in held: t.start()
for r in ready: r.wait()
print("during active:", threading.active_count())
print("enumerate names:", sorted(t.name for t in threading.enumerate()))
me = threading.current_thread()
print("main is current:", me is threading.main_thread())
print("current type/name/daemon:", type(me).__name__, me.name, me.daemon)
print("main repr head:", repr(threading.main_thread()).split(",")[0] + ">")
print("held repr head:", repr(held[0]).split(",")[0] + ">")
print("ident positive:", threading.get_ident() > 0, threading.get_native_id() > 0)
gate.set()
for t in held: t.join()
print("after join active:", threading.active_count())
print("joined alive flags:", [t.is_alive() for t in held])
