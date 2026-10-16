# Cites: S-02-20 (lazy start), S-02-21 (send value at yield), S-02-22 (StopIteration.value), C-31
# Stresses: one generator resumed 20k times with sends that alternate int/str/None mid-run:
# resume-point state and the yield-expression value must survive deopt at every resume.
def source():
    total_n = 0
    total_s = 0
    for i in range(20000):
        v = yield i
        if isinstance(v, int):
            total_n += v
        elif isinstance(v, str):
            total_s += len(v)
    return total_n, total_s

g = source()
count = 0
total = 0
send_val = None
result = None
while True:
    try:
        v = g.send(send_val)
    except StopIteration as e:
        result = e.value
        break
    total += v
    count += 1
    send_val = total if count % 2 == 0 else "z" * (count % 5 + 1)
print(count)
print(result)
print(total % 1000003)
