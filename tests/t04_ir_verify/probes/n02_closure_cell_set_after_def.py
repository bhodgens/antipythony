def outer():
    def inner():
        return a
    a = 1
    return inner

print(outer()())
# cites: S-02-12 cell defined before call -> verify silent | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
