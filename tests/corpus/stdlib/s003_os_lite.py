# Cites: S-05-4 (spec 05 §5.3.1 row os os-lite), C-05 floor
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import os
subset = ["sep", "pathsep", "curdir", "pardir", "extsep", "altsep", "name",
          "linesep", "environ", "environb", "getenv", "putenv", "unsetenv",
          "getcwd", "getcwdb", "listdir", "scandir", "stat", "lstat",
          "stat_result", "statvfs", "mkdir", "makedirs", "rmdir",
          "removedirs", "rename", "replace", "remove", "unlink", "walk",
          "access", "F_OK", "R_OK", "W_OK", "X_OK", "SEEK_SET", "SEEK_CUR",
          "SEEK_END", "open", "close", "read", "write", "lseek", "fdopen",
          "chmod", "umask", "fsdecode", "fsencode", "fspath", "PathLike",
          "error", "getpid", "getppid", "strerror", "devnull", "TMP_MAX",
          "cpu_count", "process_cpu_count"]
print(all(hasattr(os, n) for n in subset))
print(repr(os.sep), repr(os.pathsep), repr(os.curdir), repr(os.pardir),
      repr(os.extsep), repr(os.altsep), os.name, repr(os.linesep))
print(os.error is OSError, os.devnull, os.SEEK_SET, os.SEEK_CUR, os.SEEK_END)
os.environ["APX_T11_KEY"] = "apxval"
print(os.getenv("APX_T11_KEY"), os.environ.get("APX_T11_MISSING", "dflt"))
del os.environ["APX_T11_KEY"]
print(os.getenv("APX_T11_KEY") is None, os.getenv("APX_T11_KEY", "d") == "d")
print(isinstance(os.environb, os.environ.__class__),
      isinstance(os.getcwd(), str), isinstance(os.getcwdb(), bytes))
print(os.fspath("a/b"), type(os.fsencode("abc")) is bytes,
      type(os.fsdecode(b"abc")) is str, os.fsdecode(os.fsencode("é")) == "é")
print(type(os.listdir(".")) is list)
print(os.getpid() > 0, os.getppid() > 0, os.TMP_MAX > 0)
print(os.cpu_count() is None or os.cpu_count() > 0,
      os.process_cpu_count() > 0)
print(len(os.strerror(2)) > 0)
print(sorted(k for k in ("O_RDONLY", "O_WRONLY", "O_RDWR", "O_CREAT")
      if hasattr(os, k)) == ["O_CREAT", "O_RDONLY", "O_RDWR", "O_WRONLY"])
print(os.access(".", os.F_OK), os.access("no_such_t11_dir", os.F_OK))
print(issubclass(os.PathLike, object), issubclass(os.error, OSError),
      isinstance(os.stat_result, type) or hasattr(os, "stat_result"))
