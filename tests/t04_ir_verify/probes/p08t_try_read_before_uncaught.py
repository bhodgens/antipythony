def f():
    try:
        print(x)
    except ZeroDivisionError:
        pass
    x = 1

f()
# cites: S-02-2 + S-02-30 clause mismatch -> error propagates | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
