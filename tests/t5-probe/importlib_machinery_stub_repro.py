"""T5 Wall-2 F2 repro: importlib.machinery/util are native stubs.

CPython 3.14.7: SourceFileLoader ctor works; spec_from_file_location gives a
real loader with exec_module.
antipythony: calling the loader class raises
  NameError: name 'SourceFileLoader' is not defined
and spec_from_file_location's loader lacks exec_module
  AttributeError: 'object' object has no attribute 'exec_module'
"""
import importlib.machinery as im
import importlib.util

print("SourceFileLoader __module__:", getattr(im.SourceFileLoader, "__module__", None))
print("own dict:", list(getattr(im.SourceFileLoader, "__dict__", {})))

try:
    loader = getattr(im, "SourceFileLoader")("m", "/tmp/m.py")
    print("ctor OK:", loader)
except BaseException as e:  # NameError on antipythony
    print("ctor FAIL:", type(e).__name__, str(e))

spec = importlib.util.spec_from_file_location(
    "te", "/opt/homebrew/lib/python3.14/site-packages/typing_extensions.py"
)
print("spec loader:", spec.loader if spec else None)
try:
    spec.loader.exec_module
    print("exec_module OK")
except BaseException as e:
    print("exec_module FAIL:", type(e).__name__, str(e))

print("DONE", flush=True)
