# Cites: S-05-4 (row inspect), anchor #10, T2
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import inspect
names = ["signature", "Signature", "Parameter", "BoundArguments",
         "FullArgSpec", "getfullargspec", "getargs", "getargvalues",
         "Arguments", "ArgInfo", "currentframe", "stack", "getouterframes",
         "getframeinfo", "FrameInfo", "Traceback", "get_annotations",
         "unwrap", "isclass", "isfunction", "ismethod", "ismodule",
         "istraceback", "isframe", "iscode", "isgenerator",
         "isgeneratorfunction", "isasyncgenfunction", "iscoroutine",
         "iscoroutinefunction", "isawaitable", "isbuiltin", "isroutine",
         "getmodule", "getmodulename", "getfile", "getabsfile",
         "getsourcefile", "getsourcelines", "getsource", "findsource",
         "getblock", "cleandoc", "getmembers", "getmembers_static",
         "getattr_static", "classify_class_attrs", "getmro",
         "getclosurevars", "ClosureVars", "formatannotation",
         "formatargvalues"]
print(len(names), all(hasattr(inspect, n) for n in names))
def f(a, b=2, *args, **kw):
    pass
print(inspect.signature(f), inspect.isfunction(f))
print(inspect.getfullargspec(f))
print([str(p) for p in inspect.signature(f).parameters.values()])
print([p.kind.name for p in inspect.signature(f).parameters.values()])
print(inspect.signature(abs), inspect.isbuiltin(abs))
def g(a: int, b: "str" = "x", *, c=3) -> bool:
    pass
print(inspect.signature(g))
print(inspect.signature(g, eval_str=True))
sig = inspect.signature(g)
bnd = sig.bind(1, "two", c=3)
print(sorted(bnd.arguments), hasattr(bnd, "args"), hasattr(bnd, "kwargs"))
def deco(fn):
    return fn
@deco
def wrapped():
    "doc"
print(inspect.unwrap(deco) is deco, wrapped.__name__)
print(inspect.cleandoc("  line1\n    line2\n\n  line3\n"))
class K:
    "cls doc"
    def m(self):
        pass
    @staticmethod
    def sm():
        pass
    @classmethod
    def cl(cls):
        pass
    prop = property(lambda self: 1)
print(inspect.isclass(K), inspect.ismethod(K().m), inspect.isfunction(K.sm))
print(inspect.getmro(K)[-1] is object, len(inspect.getmro(K)))
print(sorted(n for n, _ in inspect.getmembers(K, inspect.isfunction))[:4])
print(len(inspect.classify_class_attrs(K)) >= 1)
print(inspect.getclosurevars(lambda: f).nonlocals is not None,
      type(inspect.getclosurevars(lambda: f)).__name__)
print(inspect.formatannotation(int), inspect.formatannotation("str"))
print(inspect.ismodule(inspect), inspect.isgenerator((x for x in [])))
print(inspect.isgeneratorfunction(f), inspect.iscoroutinefunction(f),
      inspect.isawaitable(1), inspect.isroutine(abs), inspect.iscode(g.__code__))
frame = inspect.currentframe()
print(inspect.isframe(frame))
print(inspect.getframeinfo(frame).function)
print(inspect.getmodulename("a/b/c.py"), inspect.getmodulename("a/b/c"))
print(inspect.getargvalues(frame).locals is not None)
print(inspect.getouterframes(frame)[0].function)
print(type(inspect.BlockFinder).__name__ != "")
