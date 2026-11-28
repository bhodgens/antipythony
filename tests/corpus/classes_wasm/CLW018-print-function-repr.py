# Cites: S-06-3
# Printing function objects must not trap (wasm-strplus defect 3).
# DIVERGENCE from CPython, recorded by design: CPython prints
# `<function NAME at 0xADDR>` / `<bound method ...>` / `<built-in function X>`
# with runtime addresses; the wasm backend prints deterministic no-address
# shapes (`<function NAME>`) so corpus pins stay byte-stable. Pins below are
# OUR documented shapes, not CPython bytes (slicing is unsupported on the
# wasm slice, so addresses cannot be stripped at pin time).
def f():
    pass
class A:
    def m(self):
        pass
g = lambda: 0
print(f)
print(A.m)
print(g)
