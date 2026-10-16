# Cites: S-03-09
# __getattr__ counts as the found entry; misses fall through to it; instance
# dict hits bypass it. Plain miss message without __getattr__ (anchor A1 shape).
class C:
    def __getattr__(self, name):
        return "gen:" + name
c = C()
c.explicit = 1
print(c.missing2, c.explicit, hasattr(c, "whatever"))
class D: pass
d = D()
try:
    d.nope
except AttributeError as e:
    print("AttributeError:", e)
print(c.missing1 == "gen:missing1", "missing1" in c.__dict__)
