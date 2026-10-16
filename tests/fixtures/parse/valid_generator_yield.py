def counter(n):
    i = 0
    while i < n:
        yield i
        i += 1

def squares(n):
    for i in counter(n):
        yield i * i
