# Cites: S-03-33
# raise X with X a class instantiates it; instance passes through as-is
# (identity preserved at the handler).
class MyErr(Exception): pass
try:
    raise MyErr
except MyErr as e:
    print(repr(e), e.args)
inst = MyErr("payload")
try:
    raise inst
except MyErr as e:
    print(e is inst, str(e))
class NoInit(Exception):
    def __init__(self):
        print("instantiated at raise")
try:
    raise NoInit
except NoInit:
    print("caught")
