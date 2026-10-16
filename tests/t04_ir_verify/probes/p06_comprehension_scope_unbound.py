def f():
    ys = [v for v in src]
    src = [1, 2]
    return ys

f()
# cites: S-02-2 + S-02-5 first iterable evaluated in enclosing scope | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
