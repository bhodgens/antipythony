"""PEP 750 t-string AST bridge probe.

Compares this runtime's ast.parse(t-string) dump against CPython 3.14
ground truth dumps (captured via python3.14, see git history).
Run: ./target/release/antipythony run --no-cache tests/probes/tstring_ast_probe.py
"""
import ast


def dump(node):
    """Single-line ast.dump(include_attributes=False) equivalent."""
    if isinstance(node, ast.AST):
        parts = []
        for f in type(node)._fields:
            try:
                v = getattr(node, f)
            except AttributeError:
                continue
            if v is None and getattr(type(node), f, None) is None:
                continue
            parts.append("%s=%s" % (f, dump(v)))
        return "%s(%s)" % (type(node).__name__, ", ".join(parts))
    if isinstance(node, list):
        return "[" + ", ".join(dump(x) for x in node) + "]"
    return repr(node)


CASES = [
    ('t"hello {name!r:>10}"',
     "Module(body=[Expr(value=TemplateStr(values=[Constant(value='hello '), "
     "Interpolation(value=Name(id='name', ctx=Load()), str='name', conversion=114, "
     "format_spec=JoinedStr(values=[Constant(value='>10')]))]))])"),
    ('msg = t"hello {name!r:>10} and {x} end"',
     "Module(body=[Assign(targets=[Name(id='msg', ctx=Store())], value=TemplateStr("
     "values=[Constant(value='hello '), Interpolation(value=Name(id='name', ctx=Load()), "
     "str='name', conversion=114, format_spec=JoinedStr(values=[Constant(value='>10')])), "
     "Constant(value=' and '), Interpolation(value=Name(id='x', ctx=Load()), str='x', "
     "conversion=-1), Constant(value=' end')]))])"),
    ('t"pre {a} mid {b!s:{width}} post"',
     "Module(body=[Expr(value=TemplateStr(values=[Constant(value='pre '), "
     "Interpolation(value=Name(id='a', ctx=Load()), str='a', conversion=-1), "
     "Constant(value=' mid '), Interpolation(value=Name(id='b', ctx=Load()), str='b', "
     "conversion=115, format_spec=JoinedStr(values=[FormattedValue(value=Name(id='width', "
     "ctx=Load()), conversion=-1)])), Constant(value=' post')]))])"),
    ('t"{x}"',
     "Module(body=[Expr(value=TemplateStr(values=[Interpolation(value=Name(id='x', "
     "ctx=Load()), str='x', conversion=-1)]))])"),
    ('t"no fields"',
     "Module(body=[Expr(value=TemplateStr(values=[Constant(value='no fields')]))])"),
    ('t""',
     "Module(body=[Expr(value=TemplateStr(values=[]))])"),
    ('t"{x=}"',
     "Module(body=[Expr(value=TemplateStr(values=[Constant(value='x='), "
     "Interpolation(value=Name(id='x', ctx=Load()), str='x', conversion=114)]))])"),
    ('t"{ a.b[1] }"',
     "Module(body=[Expr(value=TemplateStr(values=[Interpolation(value=Subscript("
     "value=Attribute(value=Name(id='a', ctx=Load()), attr='b', ctx=Load()), "
     "slice=Constant(value=1), ctx=Load()), str=' a.b[1]', conversion=-1)]))])"),
    ('t"{x!a}"',
     "Module(body=[Expr(value=TemplateStr(values=[Interpolation(value=Name(id='x', "
     "ctx=Load()), str='x', conversion=97)]))])"),
    ('t"{y!s}"',
     "Module(body=[Expr(value=TemplateStr(values=[Interpolation(value=Name(id='y', "
     "ctx=Load()), str='y', conversion=115)]))])"),
    ('t"a" t"b {x}"',
     "Module(body=[Expr(value=TemplateStr(values=[Constant(value='ab '), "
     "Interpolation(value=Name(id='x', ctx=Load()), str='x', conversion=-1)]))])"),
    ('t"{ d[0] }"',
     "Module(body=[Expr(value=TemplateStr(values=[Interpolation(value=Subscript("
     "value=Name(id='d', ctx=Load()), slice=Constant(value=0), ctx=Load()), "
     "str=' d[0]', conversion=-1)]))])"),
    ('t"{a=:{w}}"',
     "Module(body=[Expr(value=TemplateStr(values=[Constant(value='a='), "
     "Interpolation(value=Name(id='a', ctx=Load()), str='a', conversion=-1, "
     "format_spec=JoinedStr(values=[FormattedValue(value=Name(id='w', ctx=Load()), "
     "conversion=-1)]))]))])"),
    ('t"{x=!r}"',
     "Module(body=[Expr(value=TemplateStr(values=[Constant(value='x='), "
     "Interpolation(value=Name(id='x', ctx=Load()), str='x', conversion=114)]))])"),
    ('t"{  a  =  }"',
     "Module(body=[Expr(value=TemplateStr(values=[Constant(value='  a  =  '), "
     "Interpolation(value=Name(id='a', ctx=Load()), str='  a', conversion=114)]))])"),
]

fails = 0
for src, expected in CASES:
    got = dump(ast.parse(src))
    if got != expected:
        fails += 1
        print("FAIL", src)
        print("  EXP:", expected)
        print("  GOT:", got)
    else:
        print("OK ", src)
print("failures:", fails, "of", len(CASES))
if fails:
    raise SystemExit(1)
