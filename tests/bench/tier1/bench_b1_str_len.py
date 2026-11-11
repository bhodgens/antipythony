
def hot(n):
    s = "abcdefghij"
    i = 0
    total = 0
    while i < n:
        total = len(s)
        i = i + 1
    return total
r = 0
for _ in range(30):
    r = r + hot(2000)
print(r + hot(400000))
