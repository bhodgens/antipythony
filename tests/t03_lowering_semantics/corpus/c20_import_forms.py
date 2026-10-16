# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-04-06
# Cites: S-04-07
# Cites: S-04-08
# Import statement binding effects: `import a.b` binds ONLY the top name; `from m import n` binds ONLY n; `as` rebinding binds ONLY the alias. Paths sys.path-relative per spec 04 anchor note.
import os
from os import path as p
import collections
print(type(p.join("a", "b")))
print("path" in dir())
print(sorted(collections.OrderedDict({"b": 1, "a": 2})))
