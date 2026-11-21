# Cites: S-02-8 (exception propagation), NEXT-S016B — per-instrument source
# lines, warnings face. A warning raised with stacklevel=2 from a dunder
# dispatched inside a multi-line operator expression attributes to the
# instrument line (BINARY_OP: left operand start; COMPARE_OP: first operand
# start). Pinned from /opt/homebrew/bin/python3.14 (3.14.7; provisional vs
# the C-01 pin 3.14.0, per the T-05/T-06 provenance-gap precedent).
import warnings


# w016a: warning from __add__ inside a multi-line BinOp.
class WAdd:
    def __add__(self, other):
        warnings.warn("w-add", UserWarning, stacklevel=2)
        return 0


wadd = WAdd()

with warnings.catch_warnings(record=True) as w:
    warnings.simplefilter("always")

    def w016a_f():
        y = [
            wadd
            +
            1]

    w016a_f()
    print("a-lineno", w[0].lineno if w else None)
    print("a-msg", str(w[0].message) if w else None)

# w016b: warning from __lt__ inside a chained Compare spread over lines.
class WLt:
    def __lt__(self, other):
        warnings.warn("w-lt", UserWarning, stacklevel=2)
        return True


wlt = WLt()

with warnings.catch_warnings(record=True) as w:
    warnings.simplefilter("always")

    def w016b_f():
        ok = (
            wlt
            <
            1)

    w016b_f()
    print("b-lineno", w[0].lineno if w else None)
    print("b-msg", str(w[0].message) if w else None)

# w016c: warning text/lineno when the whole expression is on ONE line but
# the enclosing display starts earlier — instrument line wins.
class WSub:
    def __sub__(self, other):
        warnings.warn("w-sub", UserWarning, stacklevel=2)
        return 0


wsub = WSub()

with warnings.catch_warnings(record=True) as w:
    warnings.simplefilter("always")

    def w016c_f():
        v = [
            wsub - 1]

    w016c_f()
    print("c-lineno", w[0].lineno if w else None)

print("done")
