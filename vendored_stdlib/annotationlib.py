"""antipythony deviation shim — annotationlib (see docs/lxml-shim.md)."""

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

    def __init__(self, arg, *, module=None, owner=None, is_class=False,
                 is_argument=True, **_w110_extra):
        # W110 (T2): typing._make_forward_ref forwards is_argument= (CPython
        # 3.14 signature). Accept and ignore it — our evaluate path doesn't
        # distinguish argument vs non-argument forward refs.
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

    def evaluate(self, globals=None, locals=None, owner=None, defaults=None,
                 format=1, emit_values=None, **_kw):
        # W110 (T2): get_type_hints(Format.FORWARDREF) evaluates simple
        # string annotations via this path. Builtins resolve through the
        # __forward_module__ (or the eval globals' builtins) — enough for
        # singledispatch's register(func) annotation probe.
        code = getattr(self, "__forward_code__", None)
        if code is None:
            raise NotImplementedError(
                "annotationlib.ForwardRef.evaluate is not supported in v0"
            )
        g = dict(globals) if globals else {}
        if "__builtins__" not in g:
            import builtins as _b
            g["__builtins__"] = _b
        l = dict(locals) if locals else {}
        # 02.08f (s026): the recorded annotation is dump_expr_inline SOURCE
        # ('int' WITH quotes for a quoted "int" annotation) — strip one
        # layer of string-literal quoting before eval so h("int") resolves.
        arg = self.__forward_arg__
        if isinstance(arg, str) and len(arg) >= 2 and arg[0] == arg[-1] and arg[0] in "'\"":
            arg = arg[1:-1]
        val = eval(arg, g, l)
        if isinstance(val, str):
            # unresolved string stays a ForwardRef under FORWARDREF format
            return self
        return val

    def __forward_evaluate__(self, *args, **kwargs):
        raise NotImplementedError(
            "annotationlib.ForwardRef evaluation is not supported in v0"
        )


def get_annotations(obj, *, globals=None, locals=None, eval_str=False,
                    annotation_format=Format.VALUE, owner=None, format=None):
    """Return obj.__annotations__ (v0: the raw dict, never evaluated).

    CPython semantics: __annotations__ on a CLASS returns that
    class's OWN annotations, never inherited ones. Our runtime's
    class-attr getattr walks the MRO, so read the class's own
    namespace dict directly and only fall back to getattr for
    non-class objects (modules/functions).
    """
    import os as _os_ga
    _ga = _os_ga.environ.get("APX_SFCDBG")
    if _ga:
        import sys as _sys_ga
        _sys_ga.stderr.write("[GA] enter %s\n" % type(obj).__name__)
    d = None
    if isinstance(obj, type):
        # antipythony (wave-80): classes (esp. typing._ProtocolMeta
        # instances) — read the OWN namespace via vars() to avoid
        # the general attr machinery (a None-descriptor call).
        try:
            d = vars(obj)
        except Exception:
            d = None
    else:
        d = getattr(obj, "__dict__", None)
    if _ga:
        _sys_ga.stderr.write("[GA] dict-ok %r\n" % (d is not None))
    if _ga:
        _sys_ga.stderr.write("[GA] pre-hasattr\n")
    if d is not None and hasattr(d, "get"):
        if _ga:
            _sys_ga.stderr.write("[GA] pre-dget\n")
        try:
            own = d.get("__annotations__")
        except Exception:
            own = None
        if own is not None:
            return own
        if isinstance(obj, type):
            # s025 (PEP 649/749): a class may carry its annotations in
            # __annotate__ (make_dataclass sets cls.__annotate__) with no
            # __annotations__ dict at all — consult the callable before
            # answering empty (the old shape returned {} and
            # _process_class then raised "'y' is a field but has no type
            # annotation" for make_dataclass-created classes).
            annotate_fn = None
            try:
                annotate_fn = d.get("__annotate__")
            except Exception:
                annotate_fn = None
            if annotate_fn is None:
                try:
                    annotate_fn = getattr(obj, "__annotate__", None)
                except Exception:
                    annotate_fn = None
            if callable(annotate_fn):
                try:
                    # pass the RESOLVED format (dataclasses calls with
                    # format=FORWARDREF; annotation_format defaults to VALUE
                    # and would trip make_dataclass's value_blocked guard).
                    res = annotate_fn(format if format is not None else annotation_format)
                    if _ga:
                        _sys_ga.stderr.write("[GA] annotate_fn ok %r\n" % (type(res).__name__,))
                    if isinstance(res, dict):
                        return res
                except Exception as _e:
                    if _ga:
                        _sys_ga.stderr.write("[GA] annotate_fn ERR %s %r\n" % (type(_e).__name__, str(_e)[:120]))
            return {}
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
    # 02.08f (s026): the object-virtual __annotate__ MARKER answers None
    # (no annotations on object) — short-circuit instead of calling it
    # (the marker name is not a callable in module scope).
    if annotate is not None and getattr(annotate, '__class__', None) is not None:
        cls = getattr(annotate, '__class__', None)
        if getattr(cls, '__name__', '') == 'function' and getattr(annotate, '__module__', '') == 'builtins':
            pass
    try:
        nm = repr(annotate)
    except Exception:
        nm = ''
    if '__object_virtual___annotate____' in nm or 'object_virtual' in nm:
        return {}
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
