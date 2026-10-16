def make():
    def get():
        return v
    if False:
        v = 1
    return get

g = make()
print(g())
