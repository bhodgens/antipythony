def f():
    try:
        x = int('nope')
    except ValueError:
        x = 0
    return x

print(f())
