# Cites: S-01-27
# Pins: only the taken branch of a conditional expression evaluates.
def pick(c):
    return 'yes' if c else 'no'
print(pick(1), pick(0), pick(''), pick([]))
x = 10
y = 20
print(x if x < y else y)
