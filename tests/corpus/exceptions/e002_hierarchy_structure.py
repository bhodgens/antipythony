# Cites: S-03-32
# Exception class hierarchy rooted at BaseException (anchor values from the
# spec's observed sorted lists; MRO chains; catch membership facts).
# `import re` is load-bearing, not decoration (issue #29): re.PatternError is
# a direct Exception subclass ONLY after re has been imported, and the pinned
# expected bytes include it. The recording reference (Homebrew CPython 3.14.7)
# pre-imports re via Homebrew's sitecustomize, and S-03-32's observed list
# presumes re-imported startup; a startup-minimal interpreter (python-build-
# standalone on CI linux — no sitecustomize) does NOT, which red-diffed the
# reference self-check (run 46064375840). Explicit import makes the case
# host-agnostic: the pin now reproduces byte-exact on ANY CPython 3.14.
import re  # noqa: F401 — ensures re.PatternError is in Exception.__subclasses__()
print(BaseException.__bases__)
print(sorted(t.__name__ for t in BaseException.__subclasses__()))
print(sorted(t.__name__ for t in Exception.__subclasses__()))
print([t.__name__ for t in ValueError.__mro__])
print([t.__name__ for t in ZeroDivisionError.__mro__])
g = ExceptionGroup("m", [ValueError("a")])
bg = BaseExceptionGroup("m2", [KeyboardInterrupt()])
try:
    raise g
except Exception as e:
    print("EG by Exception:", type(e).__name__)
try:
    raise bg
except Exception:
    print("unreached")
except BaseException as e:
    print("BEG by BaseException:", type(e).__name__)
print(tuple(
    issubclass(*p)
    for p in (
        (Exception, BaseException),
        (KeyboardInterrupt, BaseException),
        (KeyboardInterrupt, Exception),
        (SystemExit, Exception),
        (ValueError, ArithmeticError),
    )
))
