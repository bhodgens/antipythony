# Cites: S-05-4 (row genericpath/posixpath: os.path sources; GAP-17 bare -S genericpath import fails circular-import)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import posixpath
names = ["join", "split", "dirname", "basename", "exists", "isdir",
         "normpath", "abspath", "isabs", "realpath", "relpath", "expanduser",
         "sep", "curdir", "pardir", "altsep", "extsep", "pathsep"]
print(all(hasattr(posixpath, n) for n in names))
print(posixpath.join("a", "b"), posixpath.split("/x/y.zip"),
      posixpath.basename("/x/y.zip"), posixpath.dirname("/x/y.zip"))
print(posixpath.normpath("/x//y/../z"), posixpath.isabs("y"),
      posixpath.altsep is None, posixpath.sep, repr(posixpath.pathsep))
print(posixpath.exists("/no/such/path/t11"),
      posixpath.isdir("/no/such/path/t11"))
print(posixpath.splitext("/a/b.tar.gz"))
import os.path
print(posixpath.join is os.path.join,
      os.path.__name__ == "posixpath")
import genericpath
print(hasattr(genericpath, "_splitext") or hasattr(genericpath, "commonprefix"))
