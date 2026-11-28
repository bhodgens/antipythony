# Cites: S-01-31 S-02-11
# Pins: `in`/`not in` over list/tuple (D12a); str lexicographic ordering via
# < (D12b) — both were Unreachable traps on main.
print(1 in [1, 2], 1 not in [1, 2])
print(1 in (1, 2), 3 not in (1, 2))
print("a" < "b", "b" >= "b", "ab" < "b", "a" < "ab")
