# Cites: S-02-15 (if), S-02-16 (while/else), S-02-17 (for protocol), S-02-18 (break/continue), C-31
# Stresses: break/continue/else control traces across OSR in nested loops; a mis-restored
# control state at the tier boundary changes the deterministic (o, tag) trace.
trace = []
outer = 0
for o in range(8):
    inner = 0
    while True:
        inner += 1
        if inner % 5 == 0:
            continue
        if inner >= 61:
            break
        if inner == 30:
            trace.append((o, "deep"))
    else:
        trace.append((o, "no-else"))
    for s in range(625):
        outer += s
        if s == 600:
            break
    else:
        trace.append((o, "for-else"))
    if outer % 3 == 0:
        trace.append((o, "div3"))
print(len(trace))
print(trace[:6])
print(outer)
