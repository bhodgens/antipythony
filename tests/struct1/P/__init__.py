"""P: partially-initialized package under re-entrant lazy import."""
import importlib

_DYNAMIC = {"OUTPUT": ("P", ".sub"), "helper": ("P", ".sub")}


def __getattr__(attr_name):
    dynamic_attr = _DYNAMIC.get(attr_name)
    if dynamic_attr is None:
        raise AttributeError(f"module {__name__!r} has no attribute {attr_name!r}")
    package, module_name = dynamic_attr
    module = importlib.import_module(module_name, package=package)
    result = getattr(module, attr_name)
    globals()[attr_name] = result
    return result
