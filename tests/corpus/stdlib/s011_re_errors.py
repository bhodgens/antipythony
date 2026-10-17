# Cites: S-05-4 (row re), C-14 message pins
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import re
def show(fn, *a, **k):
    try:
        v = fn(*a, **k)
        return f"value {v!r}"
    except Exception as e:
        return f"{type(e).__name__}: {e}"
print(show(re.compile, "("))
print(show(re.compile, "["))
print(show(re.compile, "*"))
print(show(re.compile, "a**"))
print(show(re.compile, "(?P<n>a)(?P<n>b)"))
print(show(re.sub, "(a)", "x\\1\\2", "a"))
try:
    re.split("x", None)
except TypeError as e:
    print("TypeError:", e)
print(re.purge() is None)
p = re.compile("(a)(b)?")
m = p.match("a")
print(m.group(2), m.group(0), m.end(0), m.span(2))
print(re.error.__module__, issubclass(re.error, Exception))
