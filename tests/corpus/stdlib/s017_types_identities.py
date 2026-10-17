# Cites: S-05-4 (row types)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import sys
import types
names = ["FunctionType", "LambdaType", "GeneratorType", "CoroutineType",
         "AsyncGeneratorType", "BuiltinFunctionType", "BuiltinMethodType",
         "MethodType", "MethodWrapperType", "MethodDescriptorType",
         "ClassMethodDescriptorType", "GetSetDescriptorType",
         "MemberDescriptorType", "WrapperDescriptorType", "CodeType",
         "FrameType", "TracebackType", "ModuleType", "CellType",
         "SimpleNamespace", "DynamicClassAttribute", "MappingProxyType",
         "NoneType", "EllipsisType", "NotImplementedType", "UnionType",
         "GenericAlias", "new_class", "prepare_class", "resolve_bases",
         "get_original_bases"]
print(len(names), all(hasattr(types, n) for n in names))
def f():
    yield 1
g = f()
print(type(f) is types.FunctionType, type(g) is types.GeneratorType,
      type(f.__code__) is types.CodeType)
print(type((lambda: 1)) is types.LambdaType,
      types.LambdaType is types.FunctionType)
print(type(len) is types.BuiltinFunctionType,
      type("".upper) is types.BuiltinMethodType)
print(type(None) is types.NoneType, type(...) is types.EllipsisType,
      type(NotImplemented) is types.NotImplementedType)
print(type(int.__add__).__name__)
ns = types.SimpleNamespace(a=1, b="x")
print(ns, ns.a, "a" in ns.__dict__, repr(types.SimpleNamespace()))
mp = types.MappingProxyType({"k": 1})
print(dict(mp), mp["k"], type(mp).__name__)
def outer():
    x = 1
    def inner():
        return x
    return inner.__closure__[0]
print(type(outer()) is types.CellType)
NC = types.new_class("NC", ())
print(NC.__name__, isinstance(NC, type))
print(types.get_original_bases(NC))
print(type(int | str) is types.UnionType, type(list[int]) is types.GenericAlias)
print(types.ModuleType("t11mod").__name__)
class DCA:
    @types.DynamicClassAttribute
    def prop(self):
        return 7
print(DCA().prop)
try:
    1 / 0
except ZeroDivisionError:
    tb = sys.exc_info()[2]
print(isinstance(tb, types.TracebackType))
class PB:
    pass
print(types.resolve_bases((PB,)) == (PB,), callable(types.prepare_class))
