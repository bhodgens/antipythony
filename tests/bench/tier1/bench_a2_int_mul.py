
def hot(n):
    acc = 0
    i = 0
    while i < n:
        acc = acc + i * 3
        i = i + 1
    return acc
r = 0
for _ in range(30):
    r = r + hot(2000)
print(r + hot(400000))
