# T-01 valid fixture: del, pass, assert statements.
# Cites: S-04-11 (del removes only the local binding — the `del` statement itself is 3.14 grammar per C-03); plan-01 Stmt list names `assert`, `del`, `pass`.
import sys
version_probe = sys.version_info
del version_probe
if True:
    pass
assert 1 + 1 == 2
assert [] == [], "empty lists are equal"
x = [1]
del x[0]
y = 1
del y
print(x)
