# T-10 threading corpus T111
# Cites: S-05-4 (threading row: Condition), C-01
# Pins: Condition.wait_for handshake — consumer only proceeds after producer
# mutates shared state under the same lock; collected values printed sorted.
# The wait/notify micro-check uses entered/notified predicates under the same
# condition, so no lost-wakeup race exists.
# Determinism: predicate rendezvous, never sleep.
import threading

cond = threading.Condition()
state = {"items": [], "closed": False}
consumed = []

def producer():
    for i in range(5):
        with cond:
            state["items"].append(i)
            cond.notify_all()
    with cond:
        state["closed"] = True
        cond.notify_all()

def consumer():
    with cond:
        cond.wait_for(lambda: state["closed"])
        consumed.extend(state["items"])

c = threading.Thread(target=consumer, name="Cons")
p = threading.Thread(target=producer, name="Prod")
c.start()
p.start()
p.join()
c.join()
print("consumed:", sorted(consumed))
print("count:", len(consumed))

single = threading.Condition()
flag = {"in_wait": False, "notified": False, "woke": False}
def waiter():
    with single:
        flag["in_wait"] = True
        single.notify_all()
        single.wait_for(lambda: flag["notified"])
        flag["woke"] = True
w = threading.Thread(target=waiter, name="W")
w.start()
with single:
    single.wait_for(lambda: flag["in_wait"])
    flag["notified"] = True
    single.notify_all()
w.join()
print("wait/notify woke:", flag["woke"])
