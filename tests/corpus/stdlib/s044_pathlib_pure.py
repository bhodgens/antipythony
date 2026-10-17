# Cites: S-05-4 (row pathlib T2 enabler): pure-string ops; absent-path FS ops
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import os
import pathlib
names = ["Path", "PurePosixPath", "PureWindowsPath", "PurePath", "PosixPath",
         "WindowsPath"]
print(len(names), all(hasattr(pathlib, n) for n in names))
pure = pathlib.PurePosixPath("/a/b.tar.gz")
print(repr(pure), pure.name, pure.stem, pure.suffix)
print(pure.parent, pure.parts, pure.as_posix(), str(pure), pure.is_absolute())
q = pure / "sub" / "c.txt"
print(repr(q), q.with_name("d.py"), q.with_suffix(".zip"))
print(pure.relative_to("/a"), list(pure.parents))
print(pathlib.PureWindowsPath("C:/x/y").name,
      pathlib.PureWindowsPath("C:/x/y").drive,
      pathlib.PureWindowsPath("C:/x/y").root)
print(pathlib.PurePosixPath("a", "b").joinpath("c"),
      pathlib.PurePosixPath("/x/../y"))
print(pathlib.PurePosixPath("/x/y").match("y"),
      pathlib.PurePosixPath("/x/y").match("*/y"))
print(isinstance(pathlib.PosixPath, type),
      issubclass(pathlib.Path, pathlib.PurePath),
      issubclass(pathlib.PosixPath, pathlib.Path))
p_absent = pathlib.Path("/definitely/not/here/t11")
print(p_absent.exists(), p_absent.is_dir(), p_absent.is_file())
print(pathlib.Path("/a/b").is_absolute(), p_absent.name, p_absent.parent)
print(hasattr(pathlib.Path("."), "__fspath__"),
      isinstance(pathlib.Path("."), os.PathLike))
print(str(pathlib.PurePosixPath("a/../b")),
      pathlib.PurePosixPath("a").__class__.__name__)
print(hasattr(pathlib.Path, "walk"), hasattr(pathlib.Path, "read_text"),
      hasattr(pathlib.Path, "write_text"), hasattr(pathlib.Path, "glob"),
      hasattr(pathlib.Path, "rglob"), hasattr(pathlib.Path, "iterdir"),
      hasattr(pathlib.Path, "mkdir"), hasattr(pathlib.Path, "touch"),
      hasattr(pathlib.Path, "unlink"), hasattr(pathlib.Path, "resolve"),
      hasattr(pathlib.Path, "absolute"), hasattr(pathlib.Path, "stat"),
      hasattr(pathlib.Path, "samefile"))
print(type(pathlib.Path.cwd()).__name__ == "PosixPath",
      pathlib.Path.home().is_absolute())
print(repr(pathlib.PurePosixPath("x.txt").with_suffix(".md")),
      pathlib.PurePosixPath("a/b").suffixes)
