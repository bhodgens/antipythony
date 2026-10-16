def make():
    def get():
        return v
    if False:
        v = 1
    return get

g = make()
print(g())
# cites: S-02-12/S-02-14 free-variable NameError variant (T-04 G-3) | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
