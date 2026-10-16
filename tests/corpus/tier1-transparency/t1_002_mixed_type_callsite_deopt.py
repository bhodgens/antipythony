# Cites: S-01-1 (int), S-01-14 (str), S-02-11 (return), C-31
# Stresses: one call site specializes on int (30k calls), is forced to deopt by str (200 calls),
# then re-specializes on int again; results of all three phases must be exact.
def double(x):
    return x + x

s_int = 0
for i in range(30000):
    s_int += double(i)

s_str = ""
for ch in "abcdefghij":
    for k in range(20):
        s_str += double(ch)

s_int2 = 0
for i in range(30000):
    s_int2 = s_int2 + double(-i)

print(s_int)
print(len(s_str), s_str[:20])
print(s_int2)
