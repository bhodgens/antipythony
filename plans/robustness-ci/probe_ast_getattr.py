import ast

# Synthetic-but-realistic module: 200 defs with bodies, so ast.parse
# produces a tree with several thousand nodes spanning many node classes.
_lines = []
for i in range(200):
    _lines.append("def f%d(a, b):" % i)
    _lines.append("    x = a + b + %d" % i)
    _lines.append("    return [x, a * b, str(x)]")
src = "\n".join(_lines)

tree = ast.parse(src)

total = 0
for _i in range(1000):
    # ast.walk drives node-type dispatch + attribute reads across the
    # whole class hierarchy; each native _ast getattr used to materialize
    # ~110 fresh class clones OUTSIDE the GC heap (residual (a)).
    for node in ast.walk(tree):
        total += len(getattr(node, "_fields", ()))
        cls = type(node)
        total += len(getattr(cls, "__name__", ""))
        total += len(getattr(ast, "AST").__name__)
    total += len(getattr(ast, "parse").__name__)

print("FIELDS-TOTAL", total)
