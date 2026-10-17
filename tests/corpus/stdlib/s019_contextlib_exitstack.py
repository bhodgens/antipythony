# Cites: S-05-4 (row contextlib): ExitStack LIFO
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import contextlib
es = contextlib.ExitStack()
es.callback(print, "cb-second")
es.callback(print, "cb-first")
es.push(contextlib.suppress(KeyError))
with es:
    print("stack-body")
print("closed")
es2 = contextlib.ExitStack()
class Res:
    def __init__(self, i):
        self.i = i
    def __enter__(self):
        print("open", self.i)
        return self
    def __exit__(self, t, v, tb):
        print("close", self.i)
        return t is KeyError
a = es2.enter_context(Res(1))
b = es2.enter_context(Res(2))
print(a.i + b.i)
es2.close()
es3 = contextlib.ExitStack()
try:
    with es3:
        es3.callback(print, "callback-runs-during-unwind")
        raise RuntimeError("out")
except RuntimeError as e:
    print("stack propagated:", e)
print(isinstance(contextlib.suppress(KeyError), contextlib.suppress))
print(isinstance(contextlib.AsyncExitStack, type),
      contextlib.ExitStack is not None)
