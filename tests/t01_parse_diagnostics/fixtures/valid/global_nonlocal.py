# T-01 valid fixture: global and nonlocal statements.
# Cites: S-02-3 (global/nonlocal — task-named clause; legal inside function scopes); plan-01 Stmt list names `global/nonlocal`.
count = 0
def bump():
    global count
    count += 1
    return count
def outer():
    total = 1
    def inner():
        nonlocal total
        total += 10
    inner()
    return total
bump()
print(count, outer())
