def f():
    nonlocal x
    x = 1

f()
# cites: S-02-4 nonlocal without enclosing binding -> SyntaxError | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
