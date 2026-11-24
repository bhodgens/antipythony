# Cites: S-02-8 (exception propagation), S-03-09 (attribute lookup via
# __getattribute__ — the dunder-slot dispatch whose instrument lines are
# pinned), NEXT-S016B — per-instrument source
# lines, mixed operators: Subscript/IfExp instruments carry their own lines
# in CPython (BINARY_SUBSCR at the OBJECT start; conditional-expression
# instrument at the condition start). Probes measured divergence for
# Subscript/IfExp shapes; UnaryOp/Attribute showed no measurable divergence
# for common shapes and were NOT extended (honest gap, session record).
# Pinned from /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the
# C-01 pin 3.14.0, per the T-05/T-06 provenance-gap precedent).
import sys


def tb_frames():
    tb = sys.exc_info()[2]
    out = []
    while tb:
        out.append(tb.tb_lineno)
        tb = tb.tb_next
    return out


# m016a: raising __getitem__; object on one line, key on the next —
# BINARY_SUBSCR instrument is the OBJECT's start line.
class SubBoom:
    def __getitem__(self, k):
        raise KeyError("boom-sub")


subboom = SubBoom()


def m016a_f():
    v = (
        subboom
        [
        1])


try:
    m016a_f()
except KeyError as e:
    print("a-msg", e)
    print("a-frames", tb_frames())

# m016b: raising condition inside an IfExp; condition on the line after
# the `1 if` opener — the instrument is the CONDITION's line.


def m016b_f():
    v = (
        1 if
        [][0]
        else 2)


try:
    m016b_f()
except IndexError:
    print("b-frames", tb_frames())

print("done")
