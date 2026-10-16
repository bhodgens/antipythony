# T-01 valid fixture: match with literal and sequence patterns.
# Cites: plan-01 Stmt list names `match`; C-03 (3.14 grammar). S-02-15 (branching family).
command = "go"
match command:
    case "go" | "run":
        action = 1
    case "stop":
        action = 2
    case _:
        action = 0
match [1, [2, 3]]:
    case [1, [x, y]]:
        total = x + y
    case []:
        total = 0
print(action, total)
