# Cites: S-05-4 (spec 05 §5.3.1 row sys), S-05-6, C-16, S-06-5
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import sys
names = ["argv", "path", "modules", "meta_path", "path_hooks",
         "path_importer_cache", "stdout", "stderr", "stdin", "platform",
         "version", "version_info", "hexversion", "api_version", "maxsize",
         "maxunicode", "byteorder", "prefix", "exec_prefix", "base_prefix",
         "base_exec_prefix", "builtin_module_names", "flags", "float_info",
         "float_repr_style", "int_info", "hash_info", "implementation",
         "exc_info", "excepthook", "displayhook", "breakpointhook", "exit",
         "getdefaultencoding", "getfilesystemencoding",
         "getfilesystemencodeerrors", "intern", "setrecursionlimit",
         "getrecursionlimit", "getswitchinterval", "setswitchinterval",
         "orig_argv", "_getframe"]
print(all(hasattr(sys, n) for n in names))
# Deterministic projections only (S-05-6: module-name registries NOT gated).
print(type(sys.argv).__name__, len(sys.argv) >= 1, type(sys.path).__name__)
print(len(sys.orig_argv) >= 1, type(sys.modules).__name__)
print(sys.float_repr_style, sys.byteorder, sys.maxunicode)
print(sys.maxsize == 2**63 - 1, sys.float_info.mant_dig)
print(sys.getdefaultencoding(), sys.getfilesystemencoding(),
      sys.getfilesystemencodeerrors())
print(sys.version_info[:2] == (3, 14), sys.version_info.releaselevel)
a = "t11-intern-probe"
print(sys.intern(a + "".join([])) is sys.intern(a))
print(sys.exc_info() == (None, None, None))
sys.setrecursionlimit(2000)
print(sys.getrecursionlimit() == 2000, isinstance(sys.flags, object))
print(type(sys.implementation).__name__ in ("module", "SimpleNamespace"),
      "int_info" in repr(sys.int_info))
sys.setswitchinterval(sys.getswitchinterval())
print(True)
print(callable(sys.excepthook), callable(sys.displayhook),
      callable(sys.breakpointhook), callable(sys.exit))
print(type(sys.stdout).__name__ != "", sys.hexversion > 0, sys.api_version >= 0)
print(isinstance(sys.meta_path, list), isinstance(sys.path_hooks, list),
      isinstance(sys.path_importer_cache, dict))
