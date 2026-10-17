# Cites: S-05-4 (spec 05 §5.3.1 row os.path), S-05-22
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import os.path as p
names = ["abspath", "basename", "dirname", "isabs", "join", "normcase",
         "normpath", "split", "splitext", "splitdrive", "commonprefix",
         "commonpath", "exists", "lexists", "isdir", "isfile", "islink",
         "ismount", "getsize", "getatime", "getmtime", "getctime", "realpath",
         "relpath", "samefile", "sameopenfile", "samestat", "expanduser",
         "expandvars", "ALLOW_MISSING", "sep", "curdir", "pardir", "extsep"]
print(all(hasattr(p, n) for n in names))
print(p.join("a", "b", "c"), p.join("/a", "/b"), p.basename("/a/b/c.txt"),
      p.dirname("/a/b/c.txt"), p.split("/a/b/c.txt"))
print(p.splitext("a.tar.gz"), p.splitext("noext"), p.splitext("/dir/.bashrc"))
print(p.splitdrive("/a/b"), p.normpath("/a//b/../c"), p.normpath("./a"),
      p.normcase("A/B"))
print(p.isabs("/a"), p.isabs("a"), p.isabs(""))
print(p.commonprefix(["/a/b/c", "/a/b/d"]), p.commonpath(["/a/b", "/a/c"]))
print(p.expanduser("~no_such_user_t11"),
      p.expandvars("no-dollar-$-here") == "no-dollar-$-here")
print(p.exists("/definitely/not/here"), p.isfile("/definitely/not/here"),
      p.isdir("/definitely/not/here"), p.islink("/definitely/not/here"),
      p.lexists("/definitely/not/here"))
print(p.relpath("/a/b/c", "/a/b"), p.relpath("x"))
print(p.split(".bashrc"), p.basename("a/"), p.dirname("a"))
print(type(p.realpath("/tmp")) is str, p.ismount("/"), p.ismount("/definitely/not"))
