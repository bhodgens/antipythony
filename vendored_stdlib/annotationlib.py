"""antipythany deviation shim — annotationlib (see docs/lxml-shim.md)."""

import types


class Format:
    VALUE = 1
    FORWARDREF = 2
    STRING = 3


class ForwardRef:
    """Stub forward reference: v0 keeps the raw text only."""

    __slots__ = ("__forward_arg__", "__forward_module__", "__forward_is_class__",
                 "__forward_value__", "__forward_evaluated__", "__owner__",
                 "__cell__")

    __forward__names__ = ()

    def __init__(self, arg, *, module=None, owner=None, is_class=False):
        if isinstance(arg, types.FunctionType):
            arg = getattr(arg, "__name__", "")
        self.__forward_arg__ = arg
        self.__forward_module__ = module
        self.__forward_is_class__ = is_class

    def __repr__(self):
        return f"ForwardRef({self.__forward_arg__!r})"

    @property
    def __forward_code__(self):
        # T1-d (typing._make_forward_ref eagerly compiles the arg text so
        # __forward_code__ probes work): v0 compiles the string.
        return compile(str(self.__forward_arg__), "<forward_ref>", "eval")

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
                    annotation_format=Format.VALUE, owner=None, format=None):
    """Return obj.__annotations__ (v0: the raw dict, never evaluated)."""
    anns = getattr(obj, "__annotations__", None)
    if anns is None:
        return {}
    return anns


def get_annotate_from_class_namespace(ns, *, owner=None):
    """v0: return the __annotations__-style dict as a callable shim."""
    anns = ns.get("__annotations__") if hasattr(ns, "get") else None
    if anns is None:
        return None

    def _annotate():
        return dict(anns)

    return _annotate


def call_annotate_function(annotate, format, *, owner=None,
                           globals=None, locals=None):
    return annotate()


def annotations_to_string(anns):
    return {k: str(v) for k, v in anns.items()}


def type_repr(value):
    """Convert a value to a format suitable for the STRING format (CPython
    parity, simplified: repr() for most objects; bare qualname for builtins)."""
    import inspect

    if isinstance(value, type) or inspect.isfunction(value):
        module = getattr(value, "__module__", "")
        qualname = getattr(value, "__qualname__", getattr(value, "__name__", "?"))
        if module == "builtins":
            return qualname
        return f"{module}.{qualname}"
    if value is Ellipsis:
        return "..."
    return repr(value)
