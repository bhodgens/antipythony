# Cites: S-03-23, S-03-24
# Fallbacks when special methods are absent: len/iter messages, legacy
# __getitem__ sequence protocol, __str__ non-string TypeError,
# str-only class keeps the default (masked) repr.
class N:
    def __getitem__(self, i):
        if i > 2:
            raise IndexError(i)
        return i
print(list(N()))
class A: pass
try:
    len(A())
except TypeError as e:
    print("TypeError:", e)
try:
    iter(A())
except TypeError as e:
    print("TypeError:", e)
class S24:
    def __str__(self):
        return 7
try:
    str(S24())
except TypeError as e:
    print("TypeError:", e)
class StrOnly:
    def __str__(self):
        return "S"
so = StrOnly()
print(str(so), repr(so).startswith("<__main__.StrOnly object at 0x"))
