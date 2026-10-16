def f():
    fs = [lambda: i for i in range(3)]
    return fs[2]()

print(f())
