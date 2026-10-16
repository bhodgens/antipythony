# Cites: S-03-32
# Exception class hierarchy rooted at BaseException (anchor values from the
# spec's observed sorted lists; MRO chains; catch membership facts).
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
