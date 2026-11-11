
def hot(n):
    s = "x"
    i = 0
    while i < n:
        s = s + "y"
        i = i + 1
    return len(s)
r = 0
for _ in range(30):
    r = r + hot(2000)
print(r + hot(30000))
