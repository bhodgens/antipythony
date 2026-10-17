# Cites: S-05-4 (row tempfile IN-lite): anchor output prohibited (paths nondeterministic) — booleans only
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import os
import tempfile
names = ["TemporaryDirectory", "NamedTemporaryFile", "mkdtemp", "gettempdir"]
print(len(names), all(hasattr(tempfile, n) for n in names))
print(callable(tempfile.mkdtemp), callable(tempfile.gettempdir))
td = tempfile.gettempdir()
print(type(td) is str, len(td) > 0, td == tempfile.gettempdir())
print(issubclass(tempfile.TemporaryDirectory, object),
      tempfile.NamedTemporaryFile is not None)
with tempfile.TemporaryDirectory(prefix="t11x-") as d:
    print(os.path.isdir(d),
          d.startswith(os.path.join(tempfile.gettempdir(), "t11x-")))
print(not os.path.isdir(d))
