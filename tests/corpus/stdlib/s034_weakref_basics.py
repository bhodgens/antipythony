# Cites: S-05-4 (row weakref), C-22/C-23 (no timing asserts)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import weakref
names = ["ref", "proxy", "ProxyType", "CallableProxyType", "ProxyTypes",
         "ReferenceType", "KeyedRef", "WeakMethod", "finalize",
         "WeakKeyDictionary", "WeakSet", "WeakValueDictionary",
         "getweakrefcount", "getweakrefs"]
print(len(names), all(hasattr(weakref, n) for n in names))
class Obj:
    def __init__(self):
        self.tag = "alive"
    def method(self):
        return "m"
o = Obj()
r = weakref.ref(o)
print(r() is o, r().tag)
proxy = weakref.proxy(o)
print(proxy.tag, isinstance(proxy, weakref.ProxyTypes))
ws = weakref.WeakSet()
ws.add(o)
print(o in ws, len(ws) >= 1)
wkd = weakref.WeakKeyDictionary()
key = Obj()
wkd[key] = 5
print(wkd[key] == 5)
wvd = weakref.WeakValueDictionary()
val = Obj()
wvd["v"] = val
print(wvd["v"].tag)
wm = weakref.WeakMethod(o.method)
print(wm()() == "m" if wm() else "gone")
f = weakref.finalize(o, lambda: None)
print(f.alive)
f.detach()
print(not f.alive)
print(r() is o)
class NonRef:
    __slots__ = ("x",)
try:
    weakref.ref(NonRef())
except TypeError as e:
    print("unweakrefable:", str(e)[:44])
print(weakref.ReferenceType is weakref.ref, weakref.ProxyType is not None)
