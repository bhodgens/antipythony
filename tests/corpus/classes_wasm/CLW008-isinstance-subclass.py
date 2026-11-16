# Cites: S-03-31
# isinstance across a user subclass chain.
class A:
    pass

class B(A):
    pass

b = B()
print(isinstance(b, B), isinstance(b, A))
