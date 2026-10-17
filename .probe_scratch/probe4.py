# extra probes for clause rewrites
def g():
    v = 1
    del v
    try:
        return v
    except Exception as e:
        print("del-then-use local:", type(e).__name__, repr(str(e)))
g()

def h():
    w = 1
    del w
    try:
        del w
    except Exception as e:
        print("double-del local:", type(e).__name__, repr(str(e)))
h()

zz = 1
del zz
try:
    zz
except Exception as e:
    print("del-then-use module:", type(e).__name__, repr(str(e)))

# nested tuple variants
for target_src in ["((KeyError, ValueError),)", "(KeyError, (ValueError,))", "((ZeroDivisionError,),)"]:
    src = f"try:\n raise KeyError()\nexcept {target_src}:\n print('caught')\n"
    try:
        exec(compile(src, "<t>", "exec"))
    except Exception as e:
        print(target_src, "->", type(e).__name__, str(e)[:70])

# flat tuple of classes still works
try:
    raise ValueError()
except (KeyError, ValueError):
    print("flat tuple OK")

# zero-arg gc functions
import gc
print("freeze None:", gc.freeze() is None, "| unfreeze None:", gc.unfreeze() is None)
