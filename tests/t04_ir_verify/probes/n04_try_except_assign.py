def f():
    try:
        x = int('nope')
    except ValueError:
        x = 0
    return x

print(f())
# cites: S-02-30 except path assigns -> verify silent | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
