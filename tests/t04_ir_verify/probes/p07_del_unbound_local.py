def f():
    del q

f()
# cites: S-02-6 function-scope form + S-02-2 del unbound local -> UnboundLocalError (T-04 G-7) | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
