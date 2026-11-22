# Leaf 10.04 differential driver: C-visible instance payloads.
# Runs byte-identical under antipythony (payload extension via build-ext,
# tp_basicsize side-registry model) and CPython 3.14 (real tp_alloc'd
# struct) — the SAME source exercising the SAME C struct fields.
from payload import Box

b = Box(30)
print("P1", b.weight)
b.scale(3)
print("P2", b.weight)
print("P3", b.scaled)

# Two instances keep INDEPENDENT C blocks.
b2 = Box(7)
print("P4", b2.weight)
b2.scale(10)
print("P5", b2.weight, b2.scaled)
print("P6", b.weight, b.scaled)  # b untouched by b2's block

# Arity error text through tp_init's ParseTuple (CPython-exact).
try:
    Box()
except TypeError as e:
    print("P7", str(e)[:60])

# The getset on a NON-payload surface raises SystemError on antipythony
# (no block) — CPython has no equivalent reachable path (struct is always
# there), so this arm is antipythony-only and NOT in the differential.
