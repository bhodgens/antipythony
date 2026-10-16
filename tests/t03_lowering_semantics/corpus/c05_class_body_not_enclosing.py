# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-5
# Cites: S-02-1
# Class bodies do NOT form an enclosing scope: method m reading class-level x raises NameError.
class C:
    x = 10
    def m(self):
        return x
try:
    C().m()
except NameError as err:
    print("NameError:", err)
