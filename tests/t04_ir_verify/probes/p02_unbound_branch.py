def f(flag):
    if flag:
        y = 1
    return y

f(False)
# cites: S-02-2 assignment in a never-taken branch still makes the name local | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
