def f():
    try:
        print(x)
    except NameError:
        pass
    x = 1

f()
