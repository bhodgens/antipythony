# Cites: S-05-4 (row linecache)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import linecache
names = ["getline", "getlines", "updatecache", "checkcache", "clearcache",
         "lazycache", "cache"]
print(len(names), all(hasattr(linecache, n) for n in names))
print(repr(linecache.getline("/definitely_absent_t11.py", 1)))
print(linecache.getlines("/definitely_absent_t11.py"))
print(linecache.updatecache("/definitely_absent_t11.py") in (True, False))
print(type(linecache.cache) is dict)
linecache.clearcache()
print(len(linecache.cache) >= 0)
lc = linecache.lazycache("/tmp/t11-not-opened.py", {})
print(callable(lc) if lc is not False else "not-lazy")
print(linecache.checkcache("/tmp/t11-not-opened.py") is None)
print(linecache.getline(__file__, 1).startswith("# Cites:"))
print(len(linecache.getlines(__file__)) > 3,
      linecache.cache is not None)
