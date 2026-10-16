# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-40
# Cites: S-02-41
# with-protocol: __enter__/__exit__ via type lookup, nested items enter forward / exit reverse, truthy __exit__ suppresses body exception (anchor A02-11).
log = []
class CM:
    def __init__(self, tag):
        self.tag = tag
    def __enter__(self):
        log.append("enter:" + self.tag)
        return self.tag
    def __exit__(self, t, v, tb):
        log.append("exit:" + self.tag + ":" + str(t and t.__name__))
        return self.tag == "swallow"
with CM("a") as a, CM("swallow") as b:
    print(a, b)
    raise ValueError("x")
print(log)
