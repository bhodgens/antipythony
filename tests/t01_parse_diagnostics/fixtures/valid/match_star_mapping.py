# T-01 valid fixture: match star patterns and mapping patterns.
# Cites: plan-01 Stmt list `match`; mapping/sequence pattern grammar (3.10+, inside C-03 pin).
match (1, 2, 3):
    case [first, *rest]:
        tail = rest
    case [*single]:
        tail = single
match {"k": 1, "j": 2}:
    case {"k": v, **others}:
        seen = (v, others)
print(tail, seen)
