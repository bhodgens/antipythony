# Cites: S-05-4 (row contextlib), anchor #9, T1
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import contextlib
import io
names = ["contextmanager", "suppress", "closing", "asynccontextmanager",
         "aclosing", "nullcontext", "ExitStack", "AsyncExitStack",
         "ContextDecorator", "AsyncContextDecorator", "redirect_stdout",
         "redirect_stderr"]
print(len(names), all(hasattr(contextlib, n) for n in names))
with contextlib.suppress(ZeroDivisionError):
    1 / 0
print("ok")
try:
    with contextlib.suppress(KeyError):
        raise ValueError("v")
except ValueError as e:
    print("not-suppressed:", e)
with contextlib.suppress(KeyError, IndexError):
    {}["nope"]
print("multi-suppress")
class F:
    closed = False
    def close(self):
        F.closed = True
with contextlib.closing(F()) as f:
    pass
print(F.closed, "closing-ok")
print(repr(contextlib.nullcontext("x").__enter__()))
with contextlib.nullcontext() as n:
    print("null-none", n)
@contextlib.contextmanager
def cm(val):
    print("enter")
    try:
        yield val
    except ValueError as e:
        print("caught-in-cm", e)
    else:
        print("no-exc")
    finally:
        print("exit")
with cm(42) as v:
    print("got", v)
with cm(1):
    raise ValueError("boom")
print("escaped")
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    print("captured")
sys_buf = io.StringIO()
with contextlib.redirect_stderr(sys_buf):
    print("err-captured")
print("cap:", repr(buf.getvalue()), "err-cap:", repr(sys_buf.getvalue()))
@contextlib.contextmanager
def nest():
    with contextlib.suppress(KeyError):
        yield "nested"
with nest() as v:
    print("nested", v)
print(type(cm(1)).__name__ != "")
