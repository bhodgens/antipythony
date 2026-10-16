# Cites: S-02-16, S-02-17, S-01-19 (list append), C-31 (OSR verify: side effects exactly once)
# Stresses: loop with per-2500 side effects crosses the OSR boundary; a duplicated or dropped
# append/accumulation at the tier transition is directly observable in len(marks) and acc.
marks = []
acc = 0
for i in range(1, 50001):
    acc += i
    if i % 2500 == 0:
        marks.append(acc)
print(len(marks))
print(acc)
print(marks[0], marks[-1], marks[len(marks) // 2])
