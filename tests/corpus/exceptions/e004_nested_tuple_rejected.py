# Cites: S-03-32, S-03-34
# NOTE CONTRACT-GAP T-06-GAP-3: S-03-34 claims "nested tuples allowed", but
# CPython 3.14.7 raises the same TypeError for ANY nested tuple target whose
# elements are valid exception classes. Pinned here: nested tuple is REJECTED.
# (Recorded from 3.14.7 per the spec-pack pin; C-01 drift governs.)
def t(label, fn):
    try:
        fn()
        print(label, "-> caught")
    except TypeError as e:
        print(label, "-> TypeError:", e)
def nested1():
    try:
        raise ValueError()
    except ((KeyError, ValueError),):
        print("nested1 body")
t("nested1", nested1)
def nested2():
    try:
        raise ValueError()
    except (KeyError, (ValueError, TypeError)):
        print("nested2 body")
t("nested2", nested2)
class Weird: pass
try:
    try:
        raise KeyError()
    except (Weird, KeyError):
        print("unreached")
except TypeError as e:
    print("nonclass-in-tuple:", type(e).__name__ + ":", e)
