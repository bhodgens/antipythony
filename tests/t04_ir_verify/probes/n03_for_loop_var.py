def f():
    for k in range(3):
        pass
    return k

print(f())
# cites: S-02-5/S-02-17 for var leaks and is bound -> verify silent | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
