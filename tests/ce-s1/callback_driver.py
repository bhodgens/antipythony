# Leaf 10.04 differential driver: the call-back bridge.
# Runs byte-identical under antipythony (callback extension via build-ext)
# and CPython 3.14 (same source compiled against Python.h).
import callback

print("R1", callback.apply(lambda x: x * 7, 6))
print("R2", callback.apply(str.upper, "shout"))
print("R3", callback.apply0(lambda: "zero-args"))
print("R4", callback.call_it(pow, (2, 10)))
print("R5", callback.vcall(lambda a, b: a + "-" + b, "left", "right"))


def double(n):
    return n * 2


print("R6", callback.apply(double, 21))

# Method callables (bound method objects) ride the same bridge.
class Greeter:
    def greet(self, name):
        return f"hi {name}"


g = Greeter()
print("R7", callback.apply(g.greet, "world"))

# The error round-trip: a raising Python fn must surface in C as NULL +
# error state, and the extension's propagate() reports 1.
def boom():
    raise ValueError("from python")


print("R8", callback.propagate(boom))

# CPython's exact TypeError for a non-callable through PyObject_CallOneArg.
try:
    callback.apply(42, 1)
except TypeError as e:
    print("R9", type(e).__name__, str(e)[:60])
