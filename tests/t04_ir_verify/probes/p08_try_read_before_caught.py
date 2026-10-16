def f():
    try:
        print(x)
    except NameError:
        pass
    x = 1

f()
# cites: S-02-2 static rule + S-02-30 except NameError catches subclass; verify must still flag | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
