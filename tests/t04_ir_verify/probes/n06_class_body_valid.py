class C:
    x = 10
    y = x + 1
    def m(self):
        return self.x

c = C()
print(c.m(), C.y)
# cites: S-02-5 class-body local + self attr -> verify silent | expectation pinned in fixtures/expected_t04.json (pinned python3.14 run; see gen_expectations.py)
