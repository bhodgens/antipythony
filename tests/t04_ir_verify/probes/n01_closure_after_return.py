def outer():
    a = 1
    def inner():
        return a
    return inner

print(outer()())
# cites: S-02-6/S-02-14 read-only closure, cell set -> verify silent | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
