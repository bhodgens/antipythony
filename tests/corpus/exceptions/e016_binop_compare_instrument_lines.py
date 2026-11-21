# Cites: S-02-8 (exception propagation), plans/antipythony/01-frontend-ir
# NEXT-S016B — per-instrument source lines (extends s016's per-call lines to
# BinOp/Compare). CPython's line table is per-instrument: BINARY_OP positions
# at the LEFT OPERAND's start line, COMPARE_OP at the FIRST operand's start.
# A raise from a dunder slot dispatched inside a multi-line operator
# expression attributes the EXECUTING frame to the instrument line, not the
# statement-start line. Pinned from /opt/homebrew/bin/python3.14 (3.14.7;
# provisional vs the C-01 pin 3.14.0, per the T-05/T-06 provenance-gap
# precedent).
import sys


def tb_frames():
    tb = sys.exc_info()[2]
    out = []
    while tb:
        out.append((tb.tb_frame.f_code.co_name, tb.tb_lineno))
        tb = tb.tb_next
    return out


# e016a: raising __radd__ mid-BinOp; display starts on the def's next line,
# left operand `1` is one line below the statement, op+right operand lower.
class RAdd:
    def __radd__(self, other):
        raise ValueError("boom-add")


radd = RAdd()


def e016a_f():
    y = [
        1
        +
        radd]


try:
    e016a_f()
except ValueError as e:
    print("a-msg", e)
    print("a-frames", tb_frames())

# e016b: ZeroDivisionError in a multi-line BinOp (no custom classes) —
# the BINARY_OP instrument line is the left operand `1`'s line.


def e016b_f():
    z = (
        100
        /
        0)


try:
    e016b_f()
except ZeroDivisionError as e:
    print("b-msg", e)
    print("b-frames", tb_frames())

# e016c: raising __eq__ in a Compare whose first operand sits one line
# below the statement start; `== c` is on the following line.


class REq:
    def __eq__(self, other):
        raise TypeError("boom-eq")


req = REq()


def e016c_f():
    w = (
        1 ==
        req)


try:
    e016c_f()
except TypeError as e:
    print("c-msg", e)
    print("c-frames", tb_frames())

print("done")
