# Cites: S-03-33
# Raising a BaseException-derived class that requires init args: the bare
# instantiation at the raise site fails with the ordinary binding TypeError
# (S-02-10 text) — NOT the "exceptions must derive" message.
class NeedsArgs(Exception):
    def __init__(self, code):
        super().__init__(f"E{code}")
        self.code = code
try:
    raise NeedsArgs
except TypeError as e:
    print("TypeError:", e)
try:
    raise NeedsArgs(7)
except NeedsArgs as e:
    print("ok:", e.code, str(e))
