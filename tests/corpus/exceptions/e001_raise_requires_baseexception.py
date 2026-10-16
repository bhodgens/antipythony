# Cites: S-03-01, S-03-30, S-03-32, S-03-33
# Exception tree structure is class structure (S-03-01 inheritance rules) and
# raise rules bind BaseException-derivedness (S-03-33). Raise a bare class ->
# instantiated; raise a non-exception -> TypeError at the raise site.
class MyError(Exception): pass
class SubError(MyError): pass
print(issubclass(SubError, MyError), issubclass(MyError, Exception))
try:
    raise SubError
except MyError as e:
    print(type(e).__name__, e.args, isinstance(e, Exception))
try:
    raise 5
except TypeError as e:
    print("TypeError:", e)
try:
    raise "boom"
except TypeError as e:
    print("TypeError:", e)
class NotExc: pass
try:
    raise NotExc()
except TypeError as e:
    print("TypeError:", e)
try:
    raise ValueError("a", 2)
except ValueError as e:
    print(str(e), repr(e), e.args)
