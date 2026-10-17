# [3] float division/modulo by zero messages
for expr in ["1.0/0.0", "1.0%0.0", "1/0", "1%0", "divmod(1.0, 0.0)"]:
    try:
        eval(expr)
    except Exception as e:
        print(expr, "->", type(e).__name__, repr(str(e)))

# [4] int->str digit-limit messages
import sys
n = 10**5000
try:
    str(n)
except Exception as e:
    print("str-form:", type(e).__name__, repr(str(e)))
try:
    int("1"*5000)
except Exception as e:
    print("int-form:", type(e).__name__, repr(str(e)))

# [5] del of unbound name
def f():
    try:
        del x
    except Exception as e:
        print("func-local:", type(e).__name__, repr(str(e)))
f()
try:
    del y
except Exception as e:
    print("module:", type(e).__name__, repr(str(e)))

# [6] gc.freeze/unfreeze return values
import gc
print("freeze:", repr(gc.freeze()))
print("unfreeze:", repr(gc.unfreeze()))

# [7] weakref targetability
import weakref
for obj in [set(), frozenset(), list(), dict(), 1, 10**40, "a", 1.0, b"a", (1,), int, type(None), object()]:
    try:
        weakref.ref(obj)
        print("weakref OK:", repr(obj)[:20] if not isinstance(obj, int) or isinstance(obj, bool) else obj, type(obj).__name__)
    except TypeError as e:
        print("weakref TypeError:", type(obj).__name__, "-", str(e)[:50])
