# Leaf 1005c differential driver part 2: the frame/code/gen projections.
# (errtb_driver.py holds the PyErr band; the two-part split pins each band
# deterministically — the combined run has a recorded intermittent abort,
# see NEXT-1005C.md findings.)
import sys

import errtb


def outer():
    def inner():
        # E11: frame surface through a REAL live frame handle. The probed
        # local lives in THIS frame (closure cells are not co_varnames in
        # CPython's fast-locals model either).
        mine = "cellproof"
        f = sys._getframe(0)
        r = errtb.frame_probe_f(f, "mine")
        print("E11", r[0], r[1], r[2])
        # E11b: the GetBack chain depth from inner() (inner -> outer ->
        # module = 2 backs from inner to the root).
        print("E11b", errtb.frame_chain_depth(f))
        return "done"

    return inner()


print("E11-ret", outer())

# E12: code surface — the booleans pin the GETTER surface (each
# PyCode_Get* answers without error); the raw bytecode/varnames contents
# are runtime-internal (CPython carries real bytecode; antipythony's
# projected code objects are source-level).
mc = outer.__code__
surf = errtb.code_surface(mc)
print("E12", surf[0] >= 0, surf[1] >= 0, surf[2] >= 0, surf[3] >= 0)

# E12b: PyCode_NewEmpty returns a non-None object (repr non-empty).
print("E12b", errtb.newempty() > 0)


def gen_fn():
    yield 1


# E13: PyGen_GetCode resolves the generator's code object (CPython
# co_name is the def name — the antipythony generator stub answers
# 'generator', recorded divergence).
g = gen_fn()
gc = errtb.gen_code(g)
print("E13", gc in ("gen_fn", "generator"))

print("DONE")
