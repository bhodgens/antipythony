# Cites: S-02-12
# Pins: late binding — the closure reads the captured variable at CALL
# time, so it observes every rebinding after creation (no snapshot): the
# closure created in iteration 0 reads the FINAL i.
def build():
    f0 = None
    i = 0
    while i < 3:
        def get():
            return i
        if i == 0:
            f0 = get
        i = i + 1
    return f0
print(build()())
