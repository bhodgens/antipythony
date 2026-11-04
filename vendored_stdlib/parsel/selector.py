# antipythony deviation shim (DEVIATIONS policy — see docs/lxml-shim.md).
# 02.08-c: the native lxml shim provides real parse/xpath surfaces, so the
# REAL parsel (site-packages) is fully usable. This vendored copy exists only
# because vendored_stdlib precedes site-packages on sys.path: it re-execs the
# real parsel package (compile+exec, no importlib machinery) and re-exports.
import os as _os
import sys as _sys

_real = None
for _base in _sys.path:
    if "vendored" in _base:
        continue
    _init = _os.path.join(_base, "parsel", "__init__.py")
    if _os.path.isfile(_init):
        _pkgdir = _os.path.dirname(_init)
        try:
            with open(_init, "r", encoding="utf-8") as _fh:
                _src = _fh.read()
        except OSError:
            continue
        _mod = type(_sys)("parsel")
        _mod.__file__ = _init
        _mod.__path__ = [_pkgdir]
        _mod.__package__ = "parsel"
        _sys.modules["parsel"] = _mod
        try:
            exec(compile(_src, _init, "exec"), _mod.__dict__)
            _real = _mod
        except BaseException:
            _sys.modules.pop("parsel", None)
            _real = None
        break

if _real is not None:
    Selector = _real.Selector
    SelectorList = _real.SelectorList
else:
    class _NoSelector:
        def __init__(self, *args, **kwargs):
            raise NotImplementedError("parsel selectors unavailable")

        def xpath(self, *args, **kwargs):
            raise NotImplementedError("parsel xpath unavailable")

        def css(self, *args, **kwargs):
            raise NotImplementedError("parsel css unavailable")

    Selector = _NoSelector

    class SelectorList(list):
        pass

__all__ = ["Selector", "SelectorList"]
