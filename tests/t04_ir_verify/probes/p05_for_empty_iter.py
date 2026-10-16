def f():
    for k in []:
        pass
    return k

f()
# cites: S-02-2 + S-02-17 for-target binding; empty iterable never binds | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
