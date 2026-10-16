# T-05 values corpus V033
# Cites: S-01-34
# Pins: non-orderable ordering comparison exact TypeError text; chained
# comparisons evaluate each operand AT MOST ONCE and short-circuit (side-effect
# log proves it); normal ordering results.
try:
    1 < "x"
except Exception as e:
    print(type(e).__name__ + ":", e)
log = []
def c(v):
    log.append(v)
    return v
print(c(1) < c(2) < c(0), log)
log.clear()
result = c(1) < c(0) < c(9)
print(result, log)
print((1, 2) < (1, 3), "abc" < "abd", 3 <= 3 < 4, 1 > 2 > 3)
