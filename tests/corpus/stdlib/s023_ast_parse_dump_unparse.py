# Cites: S-05-4 (row ast), S-05-11 adjacency
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import ast
names = ["parse", "unparse", "walk", "dump", "literal_eval", "NodeVisitor",
         "NodeTransformer", "fix_missing_locations", "increment_lineno",
         "get_docstring", "get_source_segment", "iter_child_nodes",
         "iter_fields", "copy_location", "Module", "Expression",
         "Interactive", "FunctionType", "PyCF_ONLY_AST"]
print(len(names), all(hasattr(ast, n) for n in names))
tree = ast.parse("x = 1 + 2\n", mode="exec")
print(ast.dump(tree))
expr = ast.parse("a.b[c]", mode="eval")
print(ast.unparse(expr))
print(ast.unparse(ast.parse("def f(a, b=2, *args, **kw):\n  return [a, b]")))
print(ast.literal_eval("{1: [2, 'three'], 'x': None}"))
try:
    ast.literal_eval("__import__('os')")
except ValueError as e:
    print("literal-eval:", e)
print(ast.get_docstring(ast.parse("'doc string here'\nx=1")))
print([type(n).__name__ for n in ast.walk(ast.parse("a[0]"))][:4])
print(list(ast.iter_fields(tree.body[0])))
print(ast.dump(ast.Expression(value=ast.Constant(1))))
print(ast.PyCF_ONLY_AST)
class V(ast.NodeVisitor):
    def __init__(self):
        self.nums = []
    def visit_Constant(self, node):
        if isinstance(node.value, int):
            self.nums.append(node.value)
v = V()
v.visit(ast.parse("z = [1, [2], 3]"))
print(v.nums)
class T(ast.NodeTransformer):
    def visit_Name(self, node):
        node.id = node.id.upper()
        return node
print(ast.unparse(T().visit(ast.parse("foo + bar"))))
print(ast.get_source_segment("hello = world",
                             ast.parse("hello = world").body[0].targets[0]))
m2 = ast.parse("def g():\n  return 1")
ast.fix_missing_locations(m2)
print(ast.unparse(m2).replace("\n", " | "))
print(isinstance(ast.parse(""), ast.Module), ast.literal_eval("(1, 'a')"))
print(ast.unparse(ast.parse("class K(Base, metaclass=M): pass")))
