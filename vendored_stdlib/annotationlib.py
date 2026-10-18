"""Minimal annotationlib shim (antipythany vendored stdlib).

The CPython 3.14 annotationlib.py uses t-strings (PEP 750 templates), which
are outside the v1 grammar subset. inspect.py imports `Format`, `ForwardRef`,
and `get_annotations` at module level; the annotation-evaluation functions
they support are not exercised by the vendored-stdlib import gates. This
shim provides the name surface with v0 semantics and is a documented gap:
format-aware annotation evaluation raises NotImplementedError.
"""

import enum
import types

__all__ = ["Format", "ForwardRef", "get_annotations", "get_type_hints"]


class Format(enum.IntEnum):
    VALUE = 1
    VALUE_WITH_FAKE_GLOBALS = 2
    FORWARDREF = 3
    STRING = 4


class ForwardRef:
    """Stub forward reference: v0 keeps the raw text only."""

    __forward__names__ = ()

    def __init__(self, arg, *, module=None, owner=None, is_class=False):
        if isinstance(arg, types.FunctionType):
            arg = getattr(arg, "__name__", "")
        self.__forward_arg__ = arg
        self.__forward_module__ = module
        self.__forward_is_class__ = is_class

    def __repr__(self):
        return f"ForwardRef({self.__forward_arg__!r})"

    def __eq__(self, other):
        return (
            isinstance(other, ForwardRef)
            and self.__forward_arg__ == other.__forward_arg__
        )

    def __hash__(self):
        return hash(self.__forward_arg__)

    def evaluate(self, *args, **kwargs):
        raise NotImplementedError(
            "annotationlib.ForwardRef.evaluate is not supported in v0"
        )

    def __forward_evaluate__(self, *args, **kwargs):
        raise NotImplementedError(
            "annotationlib.ForwardRef evaluation is not supported in v0"
        )


def get_annotations(obj, *, globals=None, locals=None, eval_str=False,
                    annotation_format=Format.VALUE, owner=None):
    """Return obj.__annotations__ (v0: the raw dict, never evaluated)."""
    anns = getattr(obj, "__annotations__", None)
    if anns is None:
        if isinstance(obj, types.ModuleType):
            anns = {}
        elif isinstance(obj, type):
            anns = {}
        else:
            raise TypeError(f"{obj!r} is not a module, class, or callable.")
    return anns


def get_type_hints(obj, *args, **kwargs):
    """v0 alias: raw annotations without evaluation."""
    return get_annotations(obj)
