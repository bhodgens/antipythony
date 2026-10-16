# Cites: S-01-1, S-01-7, S-01-14, S-01-19, S-01-20, S-03-25 (binop/reflected dispatch), C-31
# Stresses: one `+` site cycled through six type pairs (int, float, str, list, tuple, mixed)
# at 10k iterations: megamorphic binop IC, repeated guard failures and deopts.
def add(a, b):
    return a + b

r_int = 0
r_float = 0.0
r_str = ""
r_list_n = 0
r_tuple_n = 0
r_mix = 0.0
for i in range(10000):
    r_int = add(r_int, i % 7)
    r_float = add(r_float, (i % 5) * 0.5)
    if i % 200 == 0:
        r_str = add(r_str, f"s{i % 9}")
        r_list_n = add(r_list_n, len(add([i], [i * 2])))
        r_tuple_n = add(r_tuple_n, len(add((i,), (i,))))
    r_mix = add(r_mix, (i % 3) + 0.25)
print(r_int, repr(r_float), len(r_str), r_list_n, r_tuple_n, repr(round(r_mix, 6)))
print(r_str[:12])
