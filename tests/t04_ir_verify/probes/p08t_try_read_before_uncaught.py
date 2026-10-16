def f():
    try:
        print(x)
    except ZeroDivisionError:
        pass
    x = 1

f()
