def outer():
    x = 1
    def f():
        global x
        nonlocal x
    f()

outer()
# cites: S-02-3 global+nonlocal same name -> SyntaxError | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
