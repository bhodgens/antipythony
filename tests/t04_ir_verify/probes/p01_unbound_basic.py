def f():
    print(z)
    z = 1

f()
# cites: S-02-2 read-before-assign local rule (exact 3.14 message template) | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
