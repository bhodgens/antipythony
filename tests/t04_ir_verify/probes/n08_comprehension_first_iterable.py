def f():
    vals = [v for v in range(n)]
    return vals

n = 5
print(f())
# cites: S-02-5/S-02-1 first iterable reads module global -> verify silent | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
