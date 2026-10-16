# Cites: S-03-05
# Calling a non-type goes through its type's call protocol (runtime variable
# form: no compile-time SyntaxWarning; S-03-05 pins the type default message).
x5 = 5
try:
    x5()
except TypeError as e:
    print("TypeError:", e)
class C: pass
c = C()
try:
    c()
except TypeError as e:
    print("TypeError:", e)
class Callable:
    def __call__(self, n):
        return n * 2
print(Callable()(3))
