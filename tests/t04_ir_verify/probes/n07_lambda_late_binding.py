def f():
    fs = [lambda: i for i in range(3)]
    return fs[2]()

print(f())
# cites: S-02-12 lambda over comprehension cell -> verify silent | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
