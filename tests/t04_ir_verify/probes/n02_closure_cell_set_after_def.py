def outer():
    def inner():
        return a
    a = 1
    return inner

print(outer()())
