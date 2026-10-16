def outer():
    x = 1
    def f():
        global x
        nonlocal x
    f()

outer()
