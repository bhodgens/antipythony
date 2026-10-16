# T-01 valid fixture: annotated assignments, chained comparisons, walrus-free expressions.
# Cites: S-01-34 (ordering/chained comparisons are real operations); plan-01 Stmt list names `ann`; type lattice seed per plan-01 design ("annotations seed it").
x: int = 5
y = x < 10 > 3
d: dict[str, list[int]] = {}
lst: list = []
def typed(a: int, b: str = "s") -> bool:
    return bool(a) and bool(b)
global_ann: float = 1.5
print(y, d, lst, typed(1), global_ann)
