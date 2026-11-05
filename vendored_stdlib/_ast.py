"""_ast: real AST node classes over the native generic tree.

The native ast bridge (Rust) walks the parser output and emits
generic dicts: {"_type": name, "lineno": .., "col_offset": ..,
"end_lineno": .., "end_col_offset": .., "<field>": value, ...}.
This module defines the class hierarchy (AST -> stmt/expr/... ->
concrete) and converts those dicts into instances so that
isinstance, _fields, and ast.walk behave like upstream.
"""


class AST:
    _fields = ()

    def __init__(self, *args, **kwargs):
        fields = type(self)._fields
        if args:
            if len(args) != len(fields):
                raise TypeError(
                    "%s constructor takes %d positional arguments"
                    % (type(self).__name__, len(fields))
                )
            for name, value in zip(fields, args):
                setattr(self, name, value)
        _positional = (
            "lineno", "col_offset", "end_lineno", "end_col_offset"
        )
        for name, value in kwargs.items():
            if name not in fields and name not in _positional:
                raise TypeError(
                    "%s has no field %r" % (type(self).__name__, name)
                )
            setattr(self, name, value)

    def __repr__(self):
        parts = []
        for name in type(self)._fields:
            try:
                parts.append("%s=%r" % (name, getattr(self, name)))
            except AttributeError:
                pass
        return "%s(%s)" % (type(self).__name__, ", ".join(parts))


class mod(AST):
    pass


class Module(mod):
    _fields = ("body",)


class Interactive(mod):
    _fields = ("body",)


class Expression(mod):
    _fields = ("body",)


class stmt(AST):
    pass


class FunctionDef(stmt):
    _fields = (
        "name", "args", "body", "decorator_list", "returns",
        "type_params",
    )


class AsyncFunctionDef(stmt):
    _fields = (
        "name", "args", "body", "decorator_list", "returns",
        "type_params",
    )


class ClassDef(stmt):
    _fields = (
        "name", "bases", "keywords", "body", "decorator_list",
        "type_params",
    )


class Return(stmt):
    _fields = ("value",)


class Delete(stmt):
    _fields = ("targets",)


class Assign(stmt):
    _fields = ("targets", "value")


class AugAssign(stmt):
    _fields = ("target", "op", "value")


class AnnAssign(stmt):
    _fields = ("target", "annotation", "value", "simple")


class For(stmt):
    _fields = ("target", "iter", "body", "orelse")


class AsyncFor(stmt):
    _fields = ("target", "iter", "body", "orelse")


class While(stmt):
    _fields = ("test", "body", "orelse")


class If(stmt):
    _fields = ("test", "body", "orelse")


class With(stmt):
    _fields = ("items", "body")


class AsyncWith(stmt):
    _fields = ("items", "body")


class Match(stmt):
    _fields = ("subject", "cases")


class Raise(stmt):
    _fields = ("exc", "cause")


class Try(stmt):
    _fields = ("body", "handlers", "orelse", "finalbody")


class TryStar(Try):
    pass


class Assert(stmt):
    _fields = ("test", "msg")


class Import(stmt):
    _fields = ("names",)


class ImportFrom(stmt):
    _fields = ("module", "names", "level")


class Global(stmt):
    _fields = ("names",)


class Nonlocal(stmt):
    _fields = ("names",)


class TypeAlias(stmt):
    _fields = ("name", "type_params", "value")


class Expr(stmt):
    _fields = ("value",)


class Pass(stmt):
    _fields = ()


class Break(stmt):
    _fields = ()


class Continue(stmt):
    _fields = ()


class expr(AST):
    pass


class BoolOp(expr):
    _fields = ("op", "values")


class NamedExpr(expr):
    _fields = ("target", "value")


class BinOp(expr):
    _fields = ("left", "op", "right")


class UnaryOp(expr):
    _fields = ("op", "operand")


class Lambda(expr):
    _fields = ("args", "body")


class IfExp(expr):
    _fields = ("test", "body", "orelse")


class Dict(expr):
    _fields = ("keys", "values")


class Set(expr):
    _fields = ("elts",)


class ListComp(expr):
    _fields = ("elt", "generators")


class SetComp(expr):
    _fields = ("elt", "generators")


class DictComp(expr):
    _fields = ("key", "value", "generators")


class GeneratorExp(expr):
    _fields = ("elt", "generators")


class Await(expr):
    _fields = ("value",)


class Yield(expr):
    _fields = ("value",)


class YieldFrom(expr):
    _fields = ("value",)


class Compare(expr):
    _fields = ("left", "ops", "comparators")


class Call(expr):
    _fields = ("func", "args", "keywords")


class JoinedStr(expr):
    _fields = ("values",)


class Constant(expr):
    _fields = ("value", "kind")


class Attribute(expr):
    _fields = ("value", "attr", "ctx")


class Subscript(expr):
    _fields = ("value", "slice", "ctx")


class Starred(expr):
    _fields = ("value", "ctx")


class Name(expr):
    _fields = ("id", "ctx")


class List(expr):
    _fields = ("elts", "ctx")


class Tuple(expr):
    _fields = ("elts", "ctx")


class Slice(expr):
    _fields = ("lower", "upper", "step")


# helper node families
class expr_context(AST):
    pass


class Load(expr_context):
    _fields = ()


class Store(expr_context):
    _fields = ()


class Del(expr_context):
    _fields = ()


class boolop(AST):
    pass


class And(boolop):
    _fields = ()


class Or(boolop):
    _fields = ()


class operator(AST):
    pass


class Add(operator):
    _fields = ()


class Sub(operator):
    _fields = ()


class Mult(operator):
    _fields = ()


class MatMult(operator):
    _fields = ()


class Div(operator):
    _fields = ()


class Mod(operator):
    _fields = ()


class Pow(operator):
    _fields = ()


class LShift(operator):
    _fields = ()


class RShift(operator):
    _fields = ()


class BitOr(operator):
    _fields = ()


class BitXor(operator):
    _fields = ()


class BitAnd(operator):
    _fields = ()


class FloorDiv(operator):
    _fields = ()


class unaryop(AST):
    pass


class Invert(unaryop):
    _fields = ()


class Not(unaryop):
    _fields = ()


class UAdd(unaryop):
    _fields = ()


class USub(unaryop):
    _fields = ()


class cmpop(AST):
    pass


class Eq(cmpop):
    _fields = ()


class NotEq(cmpop):
    _fields = ()


class Lt(cmpop):
    _fields = ()


class LtE(cmpop):
    _fields = ()


class Gt(cmpop):
    _fields = ()


class GtE(cmpop):
    _fields = ()


class Is(cmpop):
    _fields = ()


class IsNot(cmpop):
    _fields = ()


class In(cmpop):
    _fields = ()


class NotIn(cmpop):
    _fields = ()


class excepthandler(AST):
    pass


class ExceptHandler(excepthandler):
    _fields = ("type", "name", "body")


class arguments(AST):
    _fields = (
        "posonlyargs",
        "args",
        "vararg",
        "kwonlyargs",
        "kw_defaults",
        "kwarg",
        "defaults",
    )


class arg(AST):
    _fields = ("arg", "annotation")


class keyword(AST):
    _fields = ("arg", "value")


class alias(AST):
    _fields = ("name", "asname")


class withitem(AST):
    _fields = ("context_expr", "optional_vars")


class comprehension(AST):
    _fields = ("target", "iter", "ifs", "is_async")


class pattern(AST):
    pass


class MatchValue(pattern):
    _fields = ("value",)


class MatchSingleton(pattern):
    _fields = ("value",)


class MatchSequence(pattern):
    _fields = ("patterns",)


class MatchMapping(pattern):
    _fields = ("keys", "patterns", "rest")


class MatchClass(pattern):
    _fields = ("cls", "patterns", "kwd_attrs", "kwd_patterns")


class MatchStar(pattern):
    _fields = ("name",)


class MatchAs(pattern):
    _fields = ("pattern", "name")


class MatchOr(pattern):
    _fields = ("patterns",)


class TypeVar(AST):
    _fields = ("name", "bound", "default_value")


class ParamSpec(AST):
    _fields = ("name", "default_value")


class TypeVarTuple(AST):
    _fields = ("name", "default_value")


class ParamSpecArgs(AST):
    _fields = ()


class ParamSpecKwargs(AST):
    _fields = ()


class TypeVarTuple(AST):
    _fields = ()


class FormattedValue(expr):
    _fields = ("value", "conversion", "format_spec")
    # CPython: format_spec defaults to None; dump() omits it then
    format_spec = None


# PEP 750 (Python 3.14): t"..." template strings
class TemplateStr(expr):
    _fields = ("values",)


class Interpolation(expr):
    _fields = ("value", "str", "conversion", "format_spec")
    # CPython: format_spec defaults to None; dump() omits it then
    format_spec = None


class match_case(AST):
    _fields = ("pattern", "guard", "body")


class type_ignore(AST):
    pass


class NodeVisitor:
    def visit(self, node):
        method = "visit_" + type(node).__name__
        visitor = getattr(self, method, self.generic_visit)
        return visitor(node)

    def generic_visit(self, node):
        for field, value in iter_fields(node):
            if isinstance(value, list):
                for item in value:
                    if isinstance(item, AST):
                        self.visit(item)
            elif isinstance(value, AST):
                self.visit(value)


class NodeTransformer(NodeVisitor):
    pass


# singleton instances for op classes (CPython exposes them as classes;
# nodes reference the CLASS itself, mirroring ast.Add usage)
_op_singletons = {}


def _singleton(name):
    cls = globals()[name]
    inst = cls()
    _op_singletons[name] = inst
    return inst


def iter_fields(node):
    try:
        fields = type(node)._fields
    except AttributeError:
        raise AttributeError(
            "iter_fields on non-AST %r (%s)"
            % (node, type(node).__name__)
        )
    for name in fields:
        try:
            yield name, getattr(node, name)
        except AttributeError:
            pass


def iter_child_nodes(node):
    for _name, value in iter_fields(node):
        if isinstance(value, AST):
            yield value
        elif isinstance(value, list):
            for item in value:
                if isinstance(item, AST):
                    yield item


def walk(node):
    todo = [node]
    while todo:
        node = todo.pop()
        yield node
        todo.extend(iter_child_nodes(node))


def dump(node, annotate_fields=True, include_attributes=False):
    if isinstance(node, str):
        return node
    fields = ", ".join(
        "%s=%s" % (f, dump(getattr(node, f), annotate_fields, include_attributes))
        for f in type(node)._fields
        if hasattr(node, f)
    )
    if annotate_fields:
        return "%s(%s)" % (type(node).__name__, fields)
    return "%s(%s)" % (type(node).__name__, fields)


_NODE_TABLE = {}
_OP_INSTANCES = {}
for _name in (
    "Module", "Interactive", "Expression",
    "FunctionDef", "AsyncFunctionDef", "ClassDef",
    "Return", "Delete", "Assign", "AugAssign", "AnnAssign",
    "For", "AsyncFor", "While", "If", "With", "AsyncWith", "Match",
    "Raise", "Try", "TryStar", "Assert", "Import", "ImportFrom",
    "Global", "Nonlocal", "Expr", "Pass", "Break", "Continue",
    "TypeAlias",
    "BoolOp", "NamedExpr", "BinOp", "UnaryOp", "Lambda", "IfExp",
    "Dict", "Set", "ListComp", "SetComp", "DictComp", "GeneratorExp",
    "Await", "Yield", "YieldFrom", "Compare", "Call", "JoinedStr",
    "Constant", "Attribute", "Subscript", "Starred", "Name",
    "List", "Tuple", "Slice",
    "Load", "Store", "Del",
    "And", "Or",
    "Add", "Sub", "Mult", "MatMult", "Div", "Mod", "Pow",
    "LShift", "RShift", "BitOr", "BitXor", "BitAnd", "FloorDiv",
    "Invert", "Not", "UAdd", "USub",
    "Eq", "NotEq", "Lt", "LtE", "Gt", "GtE", "Is", "IsNot",
    "In", "NotIn",
    "ExceptHandler",
    "arguments", "arg", "keyword", "alias", "withitem",
    "comprehension", "FormattedValue", "JoinedStr",
    "TemplateStr", "Interpolation",
    "match_case",
    "pattern", "MatchValue", "MatchSingleton", "MatchSequence",
    "MatchMapping", "MatchClass", "MatchStar", "MatchAs", "MatchOr",
    "TypeVar", "ParamSpec", "TypeVarTuple",
):
    _NODE_TABLE[_name] = globals()[_name]


def _convert(d):
    """Generic dict tree (from the native bridge) -> AST instances."""
    if isinstance(d, AST):
        return d
    if isinstance(d, list):
        return [_convert(x) for x in d]
    if not isinstance(d, dict):
        return d
    tname = d["_type"]
    if tname == "__ElseBlock__":
        # native bare-else marker: return the body statements directly
        return _convert(d.get("body", []))
    cls = _NODE_TABLE.get(tname)
    if cls is None:
        raise TypeError("_convert: unknown AST node type %r" % tname)
    kwargs = {}
    for key, value in d.items():
        if key.startswith("_"):
            continue
        kwargs[key] = _convert(value)
    # op/ctx fields: the native side emits the op NAME; map to a
    # singleton INSTANCE (CPython: node.op is ast.Add() etc). The
    # instance is cached per class; walk visits it as a child node
    # matching CPython walk counts.
    for opfield in ("op", "ctx"):
        if opfield in kwargs and isinstance(kwargs[opfield], str):
            opcls = _NODE_TABLE.get(kwargs[opfield])
            if opcls is not None:
                kwargs[opfield] = _OP_INSTANCES.setdefault(
                    kwargs[opfield], opcls()
                )
    # Compare.ops arrives as a LIST of op-name strings
    if "ops" in kwargs and isinstance(kwargs["ops"], list):
        converted = []
        for entry in kwargs["ops"]:
            if isinstance(entry, str):
                opcls = _NODE_TABLE.get(entry)
                converted.append(
                    _OP_INSTANCES.setdefault(entry, opcls())
                    if opcls is not None
                    else entry
                )
            else:
                converted.append(entry)
        kwargs["ops"] = converted
    inst = cls(**kwargs)
    # JoinedStr: merge adjacent Constant values (CPython folds
    # adjacent literal parts of implicitly concatenated f-strings)
    if tname in ("JoinedStr", "TemplateStr") and "values" in kwargs:
        vals = kwargs["values"]
        if isinstance(vals, list):
            merged = []
            for v in vals:
                if (
                    isinstance(v, Constant)
                    and isinstance(getattr(v, "value", None), str)
                    and merged
                    and isinstance(merged[-1], Constant)
                    and isinstance(getattr(merged[-1], "value", None), str)
                ):
                    merged[-1].value = merged[-1].value + v.value
                else:
                    merged.append(v)
            kwargs["values"] = merged
            inst.values = merged
    return inst


def from_native(tree):
    """Entry: generic dict tree from the native bridge -> Module AST."""
    return _convert(tree)


PyCF_ONLY_AST = 0x400
PyCF_ALLOW_TOP_LEVEL_AWAIT = 0x1000
PyCF_TYPE_COMMENTS = 0x1000 << 1
PyCF_OPTIMIZED_AST = 0x1000 << 2
