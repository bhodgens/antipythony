class C:
    x = 10
    def m(self):
        return x

C().m()
# cites: S-02-5 class body not an enclosing scope for methods | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
