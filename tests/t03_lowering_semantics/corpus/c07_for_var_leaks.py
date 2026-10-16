# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-5
# Cites: S-02-17
# for-loop variables DO leak into the enclosing function scope: after the loop k == 2 (S-02-5 Observed).
def f():
    for k in range(3):
        pass
    return k
print(f())
