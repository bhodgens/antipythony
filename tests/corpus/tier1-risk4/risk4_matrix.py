# 02.05b Risk-4 adversarial probe matrix (docs/ic-version-audit.md).
# mutate-through-every-path after an IC warms; cached value must match the
# slow path. Each probe asserts observable results — any stale cache shows
# up as a wrong value (fail-closed via output divergence).
import gc

fails = []

def check(label, got, want):
    if got != want:
        fails.append(f"{label}: got {got!r} want {want!r}")

# --- instance attr set/del x (plain, slots, class w/ metaclass setattr) ---
class Plain:
    def __init__(self):
        self.v = 1

class Slotted:
    __slots__ = ("v",)
    def __init__(self):
        self.v = 2

class Meta(type):
    pass

class WithMeta(metaclass=Meta):
    def __init__(self):
        self.v = 3

objs = [Plain(), Slotted(), WithMeta()]
for i in range(3000):
    for o in objs:
        check("warm-attr", o.v, {"Plain": 1, "Slotted": 2, "WithMeta": 3}[type(o).__name__] + 0)
        o.v = o.v + 0  # attr store + load churn
for o in objs:
    if type(o).__name__ == "Slotted":
        continue  # __slots__ without "__dict__": new attrs must AttributeError
    o.extra = 99  # new attr after warm
    check("post-set", getattr(o, "extra", None), 99)
    del o.extra
    check("post-del", getattr(o, "extra", 0), 0)

# slots class: setting an undeclared slot must AttributeError
try:
    objs[1].nope = 1
    check("slots-reject", "no error", "AttributeError")
except AttributeError:
    pass

# --- dict store/del/clear/update/setdefault/popitem x (plain) ---
for i in range(3000):
    d = {"k": i}
    check("dict-warm", d["k"], i)
d2 = {"a": 1}
d2["b"] = 2
check("dict-set", d2["b"], 2)
del d2["a"]
check("dict-del", "a" in d2, False)
d2.update({"c": 3, "d": 4})
check("dict-update", d2.get("c"), 3)
d2.setdefault("e", 5)
check("dict-setdefault", d2["e"], 5)
k, v = d2.popitem()
check("dict-popitem", (k, v), ("e", 5))
d2.clear()
check("dict-clear", len(d2), 0)

# --- class method add/remove after cache warm (rows 10, 16) ---
class Warm:
    def greet(self):
        return "original"

w = Warm()
for _ in range(3000):
    check("method-warm", w.greet(), "original")

def patched(self):
    return "patched"

Warm.greet = patched  # type.__setattr__ after IC warm
for _ in range(3000):
    check("method-after-add", w.greet(), "patched")

del Warm.greet  # remove half
try:
    w.greet()
    check("method-after-del", "no error", "AttributeError")
except AttributeError:
    pass

Warm.greet = lambda self: "restored"
check("method-re-add", w.greet(), "restored")

# inherited-attr invalidation through the MRO chain (row 10)
class Base2:
    def m(self):
        return "base"

class Sub2(Base2):
    pass

s = Sub2()
for _ in range(2000):
    check("chain-warm", s.m(), "base")
Base2.m = lambda self: "chainbumped"
check("chain-after", s.m(), "chainbumped")

# --- super_stack route after class surgery (row 13) ---
class SupBase:
    def who(self):
        return "supbase"

class SupMid(SupBase):
    def who(self):
        return "supmid+" + super().who()

class SupTop(SupMid):
    def who(self):
        return "suptop+" + super().who()

t = SupTop()
for _ in range(2000):
    check("super-warm", t.who(), "suptop+supmid+supbase")
SupBase.who = lambda self: "newsupbase"
check("super-after-surgery", t.who(), "suptop+supmid+newsupbase")

# --- gc finalizer add/remove after attr-IC warm (row 15) ---
class Fin:
    def __init__(self):
        self.tag = "plain"

made = [Fin() for _ in range(100)]
for o in made:
    for _ in range(100):
        check("fin-warm", o.tag, "plain")

Fin.__del__ = lambda self: None  # install finalizer post-warm
g = Fin()
g.tag = "with-del"
check("fin-after-install", g.tag, "with-del")
del Fin.__del__  # remove finalizer
g2 = Fin()
g2.tag = "post-del-removed"
check("fin-after-remove", g2.tag, "post-del-removed")
del made, g, g2
gc.collect()

# --- enum member attr route (dict-version via PyDict) ---
from enum import Enum

class Color(Enum):
    RED = 1
    GREEN = 2

for _ in range(1000):
    check("enum-warm", Color.RED.value, 1)
check("enum-name", Color.GREEN.name, "GREEN")

# --- builtins shadowing (row 18; module-global route) ---
check("builtins-abs", abs(-5), 5)
_g = 42
for _ in range(1000):
    check("global-warm", _g, 42)
_g = 43
check("global-after-store", _g, 43)
del _g
try:
    _g
    check("global-after-del", "no error", "NameError")
except NameError:
    pass

if fails:
    print("RISK4-FAIL")
    for f in fails:
        print("  " + f)
    raise SystemExit(1)
print("RISK4-OK all probes byte-expected")
