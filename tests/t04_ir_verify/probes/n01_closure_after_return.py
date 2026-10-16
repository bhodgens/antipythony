def outer():
    a = 1
    def inner():
        return a
    return inner

print(outer()())
