# Cites: S-01-1 (unary operators), NEXT-S016C — per-instrument source lines,
# UnaryOp: CPython's UNARY_* opcodes are instrumented and position at the
# OPERATOR line (== node start). Probe u1 (PROBES-S016B recipe s4b): `-`
# alone on its own line, operand on the next -> the warning/raise from
# __neg__ attributes to the OPERATOR line, not the statement or operand
# line. Operand CALLs keep their own lines (s016 contract; probe u2).
# Pinned from /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the
# C-01 pin 3.14.0, per the T-05/T-06 provenance-gap precedent).
import sys
import warnings


def tb_frames():
    tb = sys.exc_info()[2]
    out = []
    while tb:
        out.append(tb.tb_lineno)
        tb = tb.tb_next
    return out


# m019a: raising __neg__ with the operator on its own line — the
# UNARY_NEGATIVE instrument is the operator line (26), not the operand
# line (27) and not the statement line (25).
class NegBoom:
    def __neg__(self):
        raise ArithmeticError("boom-neg")


negboom = NegBoom()


def m019a_f():
    return (
        -
        negboom
    )


try:
    m019a_f()
except ArithmeticError as e:
    print("a-msg", e)
    print("a-frames", tb_frames())

# m019b: warning from __neg__, operator alone on its own line inside a
# larger display — stacklevel=2 attributes to the operator line.
class NegWarn:
    def __neg__(self):
        warnings.warn("neg-warn", stacklevel=2)
        return 3


negwarn = NegWarn()


def m019b_f():
    with warnings.catch_warnings(record=True) as rec:
        warnings.simplefilter("always")
        v = (
            -
            negwarn
        )
        return [(str(r.message), r.lineno) for r in rec]


print("b", m019b_f())

# m019c: operand-call line is preserved (the CALL instrument, not the
# unary operator) — call on its own line warns at the call line.
class CallWarn:
    def __call__(self):
        warnings.warn("call-warn", stacklevel=2)
        return 7


callwarn = CallWarn()


def m019c_f():
    with warnings.catch_warnings(record=True) as rec:
        warnings.simplefilter("always")
        v = -(
            callwarn()
        )
        return [(str(r.message), r.lineno) for r in rec]


print("c", m019c_f())

print("done")
