# Cites: S-05-4 (row importlib basics); spec 04 owns full import semantics
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import importlib
import importlib.machinery
import importlib.util
print(hasattr(importlib, "import_module"), hasattr(importlib, "reload"),
      hasattr(importlib.util, "find_spec"),
      all(hasattr(importlib.machinery, n) for n in
          ("SourceFileLoader", "BuiltinImporter", "ExtensionFileLoader")))
m = importlib.import_module("json")
print(m.__name__, m is __import__("json"))
mp = importlib.import_module("os.path")
print(mp.__name__, mp is __import__("os").path)
pkg = importlib.import_module("collections")
print(pkg.__name__ == "collections")
import json
rj = importlib.reload(json)
print(rj.__name__ == "json")
print(importlib.util.find_spec("quixotic_mod_t11") is None)
spec = importlib.util.find_spec("json")
print(spec.name, spec is not None)
print(issubclass(importlib.machinery.SourceFileLoader, object))
print("metadata" not in dir(importlib))
