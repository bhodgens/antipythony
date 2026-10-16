# Cites: S-03-34
# Except-target validation happens at catch time: a non-BaseException class
# raises TypeError instead of silently skipping (anchor A12); plain tuple
# targets match any element.
class NotExc: pass
try:
    try:
        raise KeyError()
    except NotExc:
        pass
except TypeError as e:
    print("TypeError:", e)
try:
    raise ValueError()
except (KeyError, ValueError):
    print("tuple any-element ok")
try:
    raise TypeError()
except (KeyError, ValueError):
    print("unreached")
except TypeError:
    print("later clause ok")
try:
    try:
        raise ValueError()
    except (KeyError, 5):
        pass
except TypeError as e:
    print("tuple-with-int:", e)
