# Cites: S-03-09 (attribute protocol), NEXT-S016C — per-instrument source
# lines, Attribute: CPython's LOAD_ATTR instrument positions at the
# ATTRIBUTE NAME line, NOT the object's start (probes q1/q2/q3, PROBES-
# S016B recipe q8): obj / dot / attr-name on separate lines all attribute
# to the attr-name line — a raise from __getattr__ dispatch reports the
# attr line. ruff's Attribute node start == obj start, so lowering must
# stamp from the attr range (session record).
# Pins tb_lineno walks (linenos only — the tb frame co_name shape
# `Cls.__getattr__` vs `__getattr__` is the documented 02.03 qualname
# envelope gap, deliberately not pinned; AttributeError message/frames
# from a user __getattr__ raising AttributeError is a separate pre-
# existing envelope divergence, also not pinned here).
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


# m020a: raising __getattr__, obj / dot / attr on three lines — the
# LOAD_ATTR instrument is the ATTR-NAME line (43), not obj (41) or the
# statement line (40).
class AttrBoom:
    def __getattr__(self, name):
        raise KeyError("no " + name)


attrboom = AttrBoom()


def m020a_f():
    return (
        attrboom
        .
        nope
    )


try:
    m020a_f()
except KeyError as e:
    print("a-msg", e)
    print("a-frames", tb_frames())

# m020b: warning from __getattr__, obj on one line and `.attr` glued on
# the next — the instrument is still the attr line.
class AttrWarn:
    def __getattr__(self, name):
        warnings.warn("attr-warn", stacklevel=2)
        raise KeyError("gone " + name)


attrwarn = AttrWarn()


def m020b_f():
    with warnings.catch_warnings(record=True) as rec:
        warnings.simplefilter("always")
        try:
            v = (
                attrwarn
                .lategone
            )
        except KeyError:
            pass
        return [(str(r.message), r.lineno) for r in rec]


print("b", m020b_f())

# m020c: same-line shape — attr line == statement line; the frame walks
# to the __getattr__ raise as usual.
def m020c_f():
    return attrboom.alsonope


try:
    m020c_f()
except KeyError as e:
    print("c-msg", e)
    print("c-frames", tb_frames())

print("done")
