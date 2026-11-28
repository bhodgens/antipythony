# Cites: S-03-09 S-07-28
# Pins: a missing attribute READ (not call — EXW013's shape) raises the
# CATCHABLE AttributeError with the FULL CPython message (audit W3: the
# read path answered null and ExprStmt dropped it — rc=0 empty). The
# message bytes ('C' + ' object has no attribute ' + name) are byte-exact.
class C:
    pass
c = C()
try:
    c.missing
except AttributeError as e:
    print('caught:', e)
