# Cites: S-03-11
# Deletion shadowing an inherited attribute reports the instance's class even
# though the name exists on the class.
class P:
    x = 1
class C(P): pass
c = C()
try:
    del c.x
except AttributeError as e:
    print("AttributeError:", e)
c.__dict__["x"] = 2
del c.x
print("after instance del, class value:", c.x, "x" in c.__dict__)
