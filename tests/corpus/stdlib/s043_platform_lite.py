# Cites: S-05-4 (row platform lite): values are documented divergence — presence only, never anchored
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import platform
names = ["system", "machine", "node", "python_version",
         "python_implementation", "processor", "uname"]
print(len(names), all(hasattr(platform, n) for n in names))
print(all(callable(getattr(platform, n)) for n in names))
u = platform.uname()
print(len(u) == 6, all(hasattr(u, a) for a in
      ("system", "node", "release", "version", "machine", "processor")))
print(type(u.system).__name__ == "str",
      platform.python_version().count(".") >= 2,
      platform.python_implementation() != "")
print(sorted(u._fields) == sorted(["system", "node", "release", "version",
                                   "machine", "processor"]))
