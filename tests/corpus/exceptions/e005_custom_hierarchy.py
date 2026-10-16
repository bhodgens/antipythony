# Cites: S-03-32, S-03-33
# Custom exception class hierarchy + args semantics at the raise site
# (message/repr/args per S-03-33's observed triple).
class Level1(Exception): pass
class Level2(Level1): pass
class Level3(Level2): pass
print([t.__name__ for t in Level3.__mro__])
try:
    raise Level3("a", 2)
except Level1 as e:
    print(type(e).__name__, str(e), repr(e), e.args)
class WithInit(Level1):
    def __init__(self, code):
        super().__init__(f"E{code}")
try:
    raise WithInit(42)
except Level1 as e:
    print(type(e).__name__, str(e), repr(e), e.args)
try:
    raise Level1
except Level1 as e:
    print("bare-class:", repr(e), e.args, str(e) == "")
